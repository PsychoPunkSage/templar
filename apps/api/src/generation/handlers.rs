//! Axum route handlers for the Generation API.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::versioning::get_current_entries;
use crate::errors::AppError;
use crate::generation::fit_scoring::FitReport;
use crate::generation::generator::{generate_resume, DraftBullet, GenerateRequest};
use crate::generation::jd_parser::{parse_jd, ParsedJD};
use crate::models::resume::{ResumeBulletRow, ResumeRow};
use crate::state::AppState;

// ────────────────────────────────────────────────────────────────────────────
// Request / Response types
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ParseJdRequest {
    pub jd_text: String,
}

#[derive(Debug, Serialize)]
pub struct ParseJdResponse {
    pub parsed_jd: ParsedJD,
}

#[derive(Debug, Deserialize)]
pub struct FitScoreRequest {
    pub user_id: Uuid,
    pub jd_text: String,
}

#[derive(Debug, Serialize)]
pub struct FitScoreResponse {
    pub fit_report: FitReport,
    pub parsed_jd: ParsedJD,
}

#[derive(Debug, Serialize)]
pub struct GenerateResponse {
    pub resume_id: Uuid,
    pub fit_report: FitReport,
    pub draft_bullets: Vec<DraftBullet>,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct ResumeDetailResponse {
    pub resume: ResumeRow,
    pub bullets: Vec<ResumeBulletRow>,
}

// ────────────────────────────────────────────────────────────────────────────
// Handlers
// ────────────────────────────────────────────────────────────────────────────

/// POST /api/v1/resumes/parse-jd
///
/// Parses a raw job description and returns structured ParsedJD.
/// Useful for previewing extraction before generating.
pub async fn handle_parse_jd(
    State(state): State<AppState>,
    Json(request): Json<ParseJdRequest>,
) -> Result<Json<ParseJdResponse>, AppError> {
    if request.jd_text.trim().is_empty() {
        return Err(AppError::Validation("jd_text cannot be empty".to_string()));
    }

    let parsed_jd = parse_jd(&request.jd_text, &state.llm).await?;

    Ok(Json(ParseJdResponse { parsed_jd }))
}

/// POST /api/v1/resumes/fit-score
///
/// Returns a fit report for the user's current context against a JD.
/// Surfaces gaps before generation so the user can decide to add context.
pub async fn handle_fit_score(
    State(state): State<AppState>,
    Json(request): Json<FitScoreRequest>,
) -> Result<Json<FitScoreResponse>, AppError> {
    if request.jd_text.trim().is_empty() {
        return Err(AppError::Validation("jd_text cannot be empty".to_string()));
    }

    let parsed_jd = parse_jd(&request.jd_text, &state.llm).await?;

    let entries = get_current_entries(&state.db, request.user_id)
        .await
        .map_err(AppError::Internal)?;

    let fit_report = state.fit_scorer.score(&entries, &parsed_jd).await?;

    Ok(Json(FitScoreResponse {
        fit_report,
        parsed_jd,
    }))
}

/// POST /api/v1/resumes/generate
///
/// Full generation pipeline: JD parse → fit score → content select → tone → LLM generate.
/// Returns draft bullets tagged with source_entry_id. Bullets are NOT yet grounded or laid out.
pub async fn handle_generate(
    State(state): State<AppState>,
    Json(request): Json<GenerateRequest>,
) -> Result<Json<GenerateResponse>, AppError> {
    if request.jd_text.trim().is_empty() {
        return Err(AppError::Validation("jd_text cannot be empty".to_string()));
    }

    let response =
        generate_resume(&state.db, &state.llm, state.fit_scorer.as_ref(), request).await?;

    Ok(Json(GenerateResponse {
        resume_id: response.resume_id,
        fit_report: response.fit_report,
        draft_bullets: response.draft_bullets,
        status: response.status,
    }))
}

/// GET /api/v1/resumes/:id
///
/// Returns the full resume row and all associated bullets from the DB.
pub async fn handle_get_resume(
    State(state): State<AppState>,
    Path(resume_id): Path<Uuid>,
) -> Result<Json<ResumeDetailResponse>, AppError> {
    let resume = sqlx::query_as::<_, ResumeRow>("SELECT * FROM resumes WHERE id = $1")
        .bind(resume_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Resume {resume_id} not found")))?;

    let bullets = sqlx::query_as::<_, ResumeBulletRow>(
        "SELECT * FROM resume_bullets WHERE resume_id = $1 ORDER BY section, id",
    )
    .bind(resume_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(ResumeDetailResponse { resume, bullets }))
}
