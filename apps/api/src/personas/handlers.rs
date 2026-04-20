use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::context::versioning::get_current_entries;
use crate::errors::AppError;
use crate::generation::hash_utils::compute_context_hash;
use crate::models::resume::PersonaRow;
use crate::personas::{
    dedup::dedup_suggestions,
    hash::compute_persona_hash,
    prompts::{build_persona_suggest_prompt, PERSONA_SUGGEST_SYSTEM},
    suggestion_cache, CreatePersonaRequest, PersonaSuggestion, SuggestPersonasQuery,
    SuggestPersonasResponse, UpdatePersonaRequest,
};
use crate::state::AppState;

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/personas?user_id={uuid}
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListPersonasQuery {
    pub user_id: Uuid,
}

pub async fn handle_list_personas(
    Query(q): Query<ListPersonasQuery>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let personas = sqlx::query_as::<_, PersonaRow>(
        "SELECT * FROM personas WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(q.user_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(serde_json::json!({ "personas": personas })))
}

// ────────────────────────────────────────────────────────────────────────────
// POST /api/v1/personas
// ────────────────────────────────────────────────────────────────────────────

pub async fn handle_create_persona(
    State(state): State<AppState>,
    Json(body): Json<CreatePersonaRequest>,
) -> Result<impl IntoResponse, AppError> {
    if body.name.trim().is_empty() {
        return Err(AppError::Validation(
            "Persona name must not be empty".to_string(),
        ));
    }

    let persona = sqlx::query_as::<_, PersonaRow>(
        r#"INSERT INTO personas (user_id, name, emphasized_tags, suppressed_tags, tone_preference, section_order)
           VALUES ($1, $2, $3, $4, $5, $6)
           RETURNING *"#,
    )
    .bind(body.user_id)
    .bind(body.name.trim())
    .bind(&body.emphasized_tags)
    .bind(&body.suppressed_tags)
    .bind(body.tone_preference.as_deref())
    .bind(body.section_order)
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(persona)))
}

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/personas/:id
// ────────────────────────────────────────────────────────────────────────────

pub async fn handle_get_persona(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let persona = sqlx::query_as::<_, PersonaRow>("SELECT * FROM personas WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound(format!("Persona {} not found", id)))?;

    Ok(Json(persona))
}

// ────────────────────────────────────────────────────────────────────────────
// PATCH /api/v1/personas/:id
// ────────────────────────────────────────────────────────────────────────────

pub async fn handle_update_persona(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(body): Json<UpdatePersonaRequest>,
) -> Result<impl IntoResponse, AppError> {
    let persona = sqlx::query_as::<_, PersonaRow>(
        r#"UPDATE personas
           SET name            = COALESCE($2, name),
               emphasized_tags = COALESCE($3, emphasized_tags),
               suppressed_tags = COALESCE($4, suppressed_tags),
               tone_preference = COALESCE($5, tone_preference),
               section_order   = COALESCE($6, section_order)
           WHERE id = $1
           RETURNING *"#,
    )
    .bind(id)
    .bind(body.name.as_deref())
    .bind(body.emphasized_tags.as_deref())
    .bind(body.suppressed_tags.as_deref())
    .bind(body.tone_preference.as_deref())
    .bind(body.section_order)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound(format!("Persona {} not found", id)))?;

    Ok(Json(persona))
}

// ────────────────────────────────────────────────────────────────────────────
// DELETE /api/v1/personas/:id
// ────────────────────────────────────────────────────────────────────────────

pub async fn handle_delete_persona(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let rows = sqlx::query("DELETE FROM personas WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();

    if rows == 0 {
        return Err(AppError::NotFound(format!("Persona {} not found", id)));
    }

    Ok(StatusCode::NO_CONTENT)
}

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/personas/suggest?user_id={uuid}[&ctx_hash=...&persona_hash=...]
//
// Returns:
//   204 No Content          — client hashes match; cached result is still fresh (no LLM call)
//   200 { suggestions, context_hash, persona_hash }  — fresh suggestions (or empty array)
// ────────────────────────────────────────────────────────────────────────────

pub async fn handle_suggest_personas(
    Query(q): Query<SuggestPersonasQuery>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    // Fetch entries and existing personas concurrently — saves one DB round-trip
    let (entries, existing_personas) = tokio::try_join!(
        async {
            get_current_entries(&state.db, q.user_id)
                .await
                .map_err(AppError::Internal)
        },
        async {
            sqlx::query_as::<_, PersonaRow>(
                "SELECT * FROM personas WHERE user_id = $1 ORDER BY created_at DESC",
            )
            .bind(q.user_id)
            .fetch_all(&state.db)
            .await
            .map_err(AppError::from)
        },
    )?;

    let context_hash = compute_context_hash(&entries);
    let persona_hash = compute_persona_hash(&existing_personas);

    // 204: client cache is still fresh — skip even DB lookup
    if matches!(
        (&q.ctx_hash, &q.persona_hash),
        (Some(ch), Some(ph)) if ch == &context_hash && ph == &persona_hash
    ) {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }

    // DB cache lookup — returns immediately if hashes match a stored result
    if let Ok(Some(cached)) =
        suggestion_cache::lookup_cache(&state.db, q.user_id, &context_hash, &persona_hash).await
    {
        return Ok(Json(SuggestPersonasResponse {
            suggestions: cached,
            context_hash,
            persona_hash,
        })
        .into_response());
    }

    // Context too thin — store empty result so subsequent refreshes also hit the cache
    if entries.len() < state.config.persona_suggest_min_entries {
        if let Err(e) =
            suggestion_cache::upsert_cache(&state.db, q.user_id, &context_hash, &persona_hash, &[])
                .await
        {
            tracing::warn!(error = %e, "persona suggestion cache upsert (empty) failed — non-fatal");
        }
        return Ok(Json(SuggestPersonasResponse {
            suggestions: vec![],
            context_hash,
            persona_hash,
        })
        .into_response());
    }

    let cfg = &state.config;
    let prompt = build_persona_suggest_prompt(
        &entries,
        &existing_personas,
        cfg.persona_suggest_top_tags,
        cfg.persona_suggest_top_entries,
        cfg.persona_suggest_max_count,
    );

    let raw = state
        .llm
        .call_json::<Vec<PersonaSuggestion>>(&prompt, PERSONA_SUGGEST_SYSTEM)
        .await
        .map_err(|e| AppError::Llm(format!("Persona suggestion failed: {e}")))?;

    let suggestions =
        dedup_suggestions(raw, &existing_personas, cfg.persona_suggest_dedup_threshold);

    // Persist to DB so future refreshes skip the LLM (non-fatal if write fails)
    if let Err(e) = suggestion_cache::upsert_cache(
        &state.db,
        q.user_id,
        &context_hash,
        &persona_hash,
        &suggestions,
    )
    .await
    {
        tracing::warn!(error = %e, "persona suggestion cache upsert failed — non-fatal");
    }

    Ok(Json(SuggestPersonasResponse {
        suggestions,
        context_hash,
        persona_hash,
    })
    .into_response())
}
