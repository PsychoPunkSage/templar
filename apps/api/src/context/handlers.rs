use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::completeness::compute_completeness_report;
use crate::context::ingest::{
    confirm_ingest, parse_and_validate, IngestConfirmRequest, IngestConfirmResponse,
    IngestPreviewResponse, IngestRequest,
};
use crate::context::versioning::{
    get_current_entries, get_entries_at_version, get_version_history,
};
use crate::errors::AppError;
use crate::models::context::{ContextEntryRow, ContextSnapshotRow};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct UserIdQuery {
    pub user_id: Uuid,
}

#[derive(Serialize)]
pub struct ContextListResponse {
    pub entries: Vec<ContextEntryRow>,
    pub completeness: crate::context::completeness::CompletenessReport,
}

/// POST /api/v1/context/ingest
pub async fn handle_ingest(
    State(state): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> Result<Json<IngestPreviewResponse>, AppError> {
    let preview = parse_and_validate(&req.raw_text, &state.llm, &state.db, req.user_id).await?;
    if !preview.impact_validation.passed {
        return Err(AppError::UnprocessableEntity(
            serde_json::to_string(&preview).unwrap_or_default(),
        ));
    }
    Ok(Json(preview))
}

/// POST /api/v1/context/ingest/confirm
pub async fn handle_ingest_confirm(
    State(state): State<AppState>,
    Json(req): Json<IngestConfirmRequest>,
) -> Result<Json<IngestConfirmResponse>, AppError> {
    let response = confirm_ingest(&state.db, &state.s3, &state.config.s3_bucket, &req).await?;
    Ok(Json(response))
}

/// GET /api/v1/context
pub async fn handle_get_context(
    State(state): State<AppState>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<ContextListResponse>, AppError> {
    let entries = get_current_entries(&state.db, params.user_id).await?;
    let completeness = compute_completeness_report(&entries);
    Ok(Json(ContextListResponse {
        entries,
        completeness,
    }))
}

/// GET /api/v1/context/health
pub async fn handle_context_health(
    State(state): State<AppState>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<crate::context::completeness::CompletenessReport>, AppError> {
    let entries = get_current_entries(&state.db, params.user_id).await?;
    Ok(Json(compute_completeness_report(&entries)))
}

/// GET /api/v1/context/history
pub async fn handle_context_history(
    State(state): State<AppState>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<Vec<ContextSnapshotRow>>, AppError> {
    let history = get_version_history(&state.db, params.user_id).await?;
    Ok(Json(history))
}

/// GET /api/v1/context/version/:v
pub async fn handle_get_version(
    State(state): State<AppState>,
    Path(v): Path<i32>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<Vec<ContextEntryRow>>, AppError> {
    let entries = get_entries_at_version(&state.db, params.user_id, v).await?;
    Ok(Json(entries))
}

#[derive(Deserialize)]
pub struct EvergreenToggle {
    pub flagged_evergreen: bool,
    pub user_id: Uuid,
}

/// PATCH /api/v1/context/entries/:id/evergreen
pub async fn handle_toggle_evergreen(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<EvergreenToggle>,
) -> Result<StatusCode, AppError> {
    let existing: Option<ContextEntryRow> = sqlx::query_as(
        "SELECT * FROM context_entries WHERE entry_id = $1 AND user_id = $2 ORDER BY version DESC LIMIT 1",
    )
    .bind(id)
    .bind(req.user_id)
    .fetch_optional(&state.db)
    .await?;

    let existing = existing.ok_or_else(|| AppError::NotFound(format!("Entry {id} not found")))?;

    // Append-only: INSERT a new version with updated evergreen flag, never UPDATE
    sqlx::query(
        r#"
        INSERT INTO context_entries
            (user_id, entry_id, version, entry_type, data, raw_text,
             recency_score, impact_score, tags, flagged_evergreen, contribution_type)
        SELECT user_id, entry_id, $1, entry_type, data, raw_text,
               recency_score, impact_score, tags, $2, contribution_type
        FROM context_entries
        WHERE entry_id = $3 AND user_id = $4
        ORDER BY version DESC
        LIMIT 1
        "#,
    )
    .bind(existing.version + 1)
    .bind(req.flagged_evergreen)
    .bind(id)
    .bind(req.user_id)
    .execute(&state.db)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}
