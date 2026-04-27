//! HTTP handlers for the Interview Prep API.
//!
//! GET  /api/v1/interview-prep/:project_id           — fetch full prep data
//! POST /api/v1/interview-prep/:project_id/trigger   — enqueue prep job
//! PUT  /api/v1/interview-prep/:project_id/company   — update company context
//! GET  /api/v1/interview-prep/:project_id/status    — status poll

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::errors::AppError;
use crate::interview_prep::{
    generator::regenerate_gap_questions,
    job::enqueue_prep_job,
    models::{
        CompanyContext, GapQuestion, PrepBullet, PrepBulletRow, PrepMeta, PrepMetaRow,
        PrepResponse, PrepStatus, PrepStatusResponse, StarScaffold,
    },
};
use crate::state::AppState;

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/interview-prep/:project_id
// ────────────────────────────────────────────────────────────────────────────

/// Returns the full prep package for a project.
/// If no prep exists yet, returns `{meta: null, bullets: []}` (200 OK).
pub async fn handle_get_prep(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    // Verify project exists (also serves as access control for now)
    let project_exists: bool =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM cv_projects WHERE id = $1)")
            .bind(project_id)
            .fetch_one(&state.db)
            .await
            .map_err(|e| AppError::Internal(anyhow::anyhow!("DB error: {e}")))?;

    if !project_exists {
        return Err(AppError::NotFound(format!(
            "Project {project_id} not found"
        )));
    }

    // Fetch meta row
    let meta_row: Option<PrepMetaRow> = sqlx::query_as::<_, PrepMetaRow>(
        "SELECT id, project_id, gap_questions, company_context, status, last_generated_at, expires_at, is_stale, created_at, updated_at
         FROM interview_prep_meta WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("DB error fetching prep meta: {e}")))?;

    // Fetch bullet rows
    let bullet_rows: Vec<PrepBulletRow> = sqlx::query_as::<_, PrepBulletRow>(
        "SELECT id, project_id, bullet_hash, bullet_text, context_entry_id, star_scaffold, questions, created_at, updated_at
         FROM interview_prep_bullets WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_all(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("DB error fetching prep bullets: {e}")))?;

    // Compute server-side is_stale by comparing stored hashes vs current resume bullets
    let current_is_stale = compute_stale_flag(&state.db, project_id, &bullet_rows).await;

    // If meta row exists and stale flag changed, update it
    if let Some(ref row) = meta_row {
        if row.is_stale != current_is_stale {
            let _ = sqlx::query(
                "UPDATE interview_prep_meta SET is_stale = $1, updated_at = NOW() WHERE project_id = $2",
            )
            .bind(current_is_stale)
            .bind(project_id)
            .execute(&state.db)
            .await;
        }
    }

    let meta = meta_row.map(|r| row_to_prep_meta(r, current_is_stale));
    let bullets = bullet_rows.into_iter().map(row_to_prep_bullet).collect();

    Ok(Json(PrepResponse { meta, bullets }))
}

// ────────────────────────────────────────────────────────────────────────────
// POST /api/v1/interview-prep/:project_id/trigger
// ────────────────────────────────────────────────────────────────────────────

/// Enqueue an interview prep job. Returns 422 if project has no current_resume_id.
pub async fn handle_trigger_prep(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    // Verify project exists and has a current resume
    let current_resume_id: Option<Uuid> = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT current_resume_id FROM cv_projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("DB error: {e}")))?
    .flatten();

    if current_resume_id.is_none() {
        return Err(AppError::Validation(
            "Project has no generated resume. Generate a resume first before preparing interview prep.".to_string(),
        ));
    }

    // Enqueue (also upserts meta to generating)
    enqueue_prep_job(&state.redis, &state.db, project_id)
        .await
        .map_err(AppError::Internal)?;

    Ok(StatusCode::ACCEPTED)
}

// ────────────────────────────────────────────────────────────────────────────
// PUT /api/v1/interview-prep/:project_id/company
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct UpdateCompanyRequest {
    pub company_name: String,
    #[serde(default)]
    pub company_stage: Option<String>,
    #[serde(default)]
    pub role_title: Option<String>,
}

/// Update the company context for a prep session and re-run gap questions.
pub async fn handle_update_company(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
    Json(body): Json<UpdateCompanyRequest>,
) -> Result<impl IntoResponse, AppError> {
    if body.company_name.trim().is_empty() {
        return Err(AppError::Validation(
            "company_name cannot be empty".to_string(),
        ));
    }

    let ctx = CompanyContext {
        company_name: body.company_name.trim().to_string(),
        company_stage: body.company_stage.filter(|s| !s.trim().is_empty()),
        role_title: body.role_title.filter(|s| !s.trim().is_empty()),
    };
    let ctx_json = serde_json::to_value(&ctx)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Serialization error: {e}")))?;

    // Upsert meta with new company context
    sqlx::query(
        r#"INSERT INTO interview_prep_meta (project_id, company_context)
           VALUES ($1, $2)
           ON CONFLICT (project_id) DO UPDATE SET
               company_context = $2,
               updated_at = NOW()"#,
    )
    .bind(project_id)
    .bind(&ctx_json)
    .execute(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("DB upsert error: {e}")))?;

    // Re-run gap questions (does NOT regenerate STAR scaffolds)
    regenerate_gap_questions(&state.db, &state.llm, project_id)
        .await
        .map_err(AppError::Internal)?;

    // Return updated meta
    let meta_row: Option<PrepMetaRow> = sqlx::query_as::<_, PrepMetaRow>(
        "SELECT id, project_id, gap_questions, company_context, status, last_generated_at, expires_at, is_stale, created_at, updated_at
         FROM interview_prep_meta WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("DB error: {e}")))?;

    let meta = meta_row.map(|r| row_to_prep_meta(r, false));
    Ok(Json(meta))
}

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/interview-prep/:project_id/status
// ────────────────────────────────────────────────────────────────────────────

pub async fn handle_get_status(
    State(state): State<AppState>,
    Path(project_id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let row: Option<PrepMetaRow> = sqlx::query_as::<_, PrepMetaRow>(
        "SELECT id, project_id, gap_questions, company_context, status, last_generated_at, expires_at, is_stale, created_at, updated_at
         FROM interview_prep_meta WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| AppError::Internal(anyhow::anyhow!("DB error: {e}")))?;

    let response = match row {
        Some(r) => PrepStatusResponse {
            status: parse_status(&r.status),
            last_generated_at: r.last_generated_at,
            expires_at: r.expires_at,
            is_stale: r.is_stale,
        },
        None => PrepStatusResponse {
            status: PrepStatus::Pending,
            last_generated_at: None,
            expires_at: None,
            is_stale: false,
        },
    };

    Ok(Json(response))
}

// ────────────────────────────────────────────────────────────────────────────
// Conversion helpers
// ────────────────────────────────────────────────────────────────────────────

fn row_to_prep_meta(row: PrepMetaRow, is_stale_override: bool) -> PrepMeta {
    let gap_questions: Vec<GapQuestion> =
        serde_json::from_value(row.gap_questions).unwrap_or_default();
    let company_context: Option<CompanyContext> = row
        .company_context
        .and_then(|v| serde_json::from_value(v).ok());

    PrepMeta {
        id: row.id,
        project_id: row.project_id,
        gap_questions,
        company_context,
        status: parse_status(&row.status),
        last_generated_at: row.last_generated_at,
        expires_at: row.expires_at,
        is_stale: is_stale_override,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn row_to_prep_bullet(row: PrepBulletRow) -> PrepBullet {
    let star_scaffold: StarScaffold = serde_json::from_value(row.star_scaffold).unwrap_or_default();
    let questions: Vec<crate::interview_prep::models::PrepQuestion> =
        serde_json::from_value(row.questions).unwrap_or_default();

    PrepBullet {
        id: row.id,
        project_id: row.project_id,
        bullet_hash: row.bullet_hash,
        bullet_text: row.bullet_text,
        context_entry_id: row.context_entry_id,
        star_scaffold,
        questions,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn parse_status(s: &str) -> PrepStatus {
    match s {
        "generating" => PrepStatus::Generating,
        "ready" => PrepStatus::Ready,
        "failed" => PrepStatus::Failed,
        _ => PrepStatus::Pending,
    }
}

/// Compute is_stale by comparing stored bullet hashes vs current resume bullets.
async fn compute_stale_flag(
    db: &sqlx::PgPool,
    project_id: Uuid,
    stored_bullets: &[PrepBulletRow],
) -> bool {
    // Get current_resume_id
    let resume_id: Option<Uuid> = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT current_resume_id FROM cv_projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
    .flatten();

    let Some(rid) = resume_id else {
        return false;
    };

    let current_texts: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT bullet_text FROM resume_bullets WHERE resume_id = $1",
    )
    .bind(rid)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    use crate::interview_prep::generator::hash_bullet;
    use std::collections::HashSet;

    let stored_hashes: HashSet<&str> = stored_bullets
        .iter()
        .map(|r| r.bullet_hash.as_str())
        .collect();
    let current_hashes: HashSet<String> = current_texts.iter().map(|t| hash_bullet(t)).collect();
    let current_hash_refs: HashSet<&str> = current_hashes.iter().map(|s| s.as_str()).collect();

    stored_hashes != current_hash_refs
}
