//! Axum route handlers for the Generation API.

use axum::{
    extract::{Path, State},
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::versioning::get_current_entries;
use crate::errors::AppError;
use crate::generation::fit_cache;
use crate::generation::fit_scoring::{FitReport, /* FitScorer,*/ LlmFitScorer};
use crate::generation::generator::{generate_resume, GenerateRequest};
use crate::generation::hash_utils;
use crate::generation::jd_parser::{parse_jd, ParsedJD};
use crate::layout::SimulatedBullet;
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
    #[serde(default)]
    pub force_refresh: bool,
}

#[derive(Debug, Serialize)]
pub struct FitScoreResponse {
    pub fit_report: FitReport,
    pub parsed_jd: ParsedJD,
    pub cache_hit: bool,
    pub jd_hash: String,
    pub context_hash: String,
}

#[derive(Debug, Serialize)]
pub struct GenerateResponse {
    pub resume_id: Uuid,
    pub fit_report: FitReport,
    /// Phase 3: bullets are now `SimulatedBullet` with `verified_line_count`,
    /// `was_adjusted`, and `flagged_for_review` populated by the simulation loop.
    pub bullets: Vec<SimulatedBullet>,
    pub status: String,
    /// True if the page fill pass could not resolve whitespace/overflow within MAX_FILL_PASSES.
    /// Frontend should surface a layout warning banner when this is true.
    pub layout_flagged: bool,
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
/// Two-step JD workflow with hash-based caching:
/// 1. Parse JD → `parsed_jd`
/// 2. Compute (jd_hash, context_hash) pair
/// 3. Cache hit (and not force_refresh) → return cached FitReport immediately
/// 4. Cache miss (or force_refresh) → run LlmFitScorer, upsert cache, return fresh score
///
/// `force_refresh = true` bypasses the cache and always re-scores via LLM.
pub async fn handle_fit_score(
    State(state): State<AppState>,
    Json(request): Json<FitScoreRequest>,
) -> Result<Json<FitScoreResponse>, AppError> {
    if request.jd_text.trim().is_empty() {
        return Err(AppError::Validation("jd_text cannot be empty".to_string()));
    }

    // Step 1: Parse JD
    let parsed_jd = parse_jd(&request.jd_text, &state.llm).await?;

    // Step 2: Fetch context entries and compute hashes
    let entries = get_current_entries(&state.db, request.user_id)
        .await
        .map_err(AppError::Internal)?;

    let jd_hash = hash_utils::compute_jd_hash(&request.jd_text);
    let context_hash = hash_utils::compute_context_hash(&entries);

    // Step 3: Cache lookup (skip if force_refresh requested)
    if !request.force_refresh {
        match fit_cache::lookup_cache(&state.db, request.user_id, &jd_hash, &context_hash).await {
            Ok(Some(cached_report)) => {
                tracing::debug!(
                    user_id = %request.user_id,
                    jd_hash = %jd_hash,
                    context_hash = %context_hash,
                    "fit-score cache hit"
                );
                return Ok(Json(FitScoreResponse {
                    fit_report: cached_report,
                    parsed_jd,
                    cache_hit: true,
                    jd_hash,
                    context_hash,
                }));
            }
            Ok(None) => {} // cache miss — fall through to LLM scoring
            Err(e) => {
                // Cache lookup failure is non-fatal — log and fall through to LLM
                tracing::warn!(error = %e, "fit-score cache lookup failed, falling through to LLM scorer");
            }
        }
    }

    // Step 4: Cache miss or force_refresh — run LlmFitScorer directly
    // We bypass `state.fit_scorer` here and use a fresh LlmFitScorer directly,
    // invoking score_full() which passes the complete untruncated raw_text to Claude.
    let scorer = LlmFitScorer(state.llm.clone());
    // DIAGNOSTIC: using score_full() — passes complete raw_text + raw JD to Claude.
    // Switch back to scorer.score() once score variation is confirmed working.
    let fit_report = scorer
        .score_full(&entries, &parsed_jd, &request.jd_text)
        .await?;

    // Step 5: Upsert cache (non-fatal on failure)
    if let Err(e) = fit_cache::upsert_cache(
        &state.db,
        request.user_id,
        &jd_hash,
        &context_hash,
        &fit_report,
    )
    .await
    {
        tracing::warn!(error = %e, "fit-score cache upsert failed (non-fatal)");
    }

    Ok(Json(FitScoreResponse {
        fit_report,
        parsed_jd,
        cache_hit: false,
        jd_hash,
        context_hash,
    }))
}

/// POST /api/v1/resumes/generate
///
/// Full generation pipeline: JD parse → fit score → content select → tone → LLM generate
/// → layout simulation → persist. Phase 3: returns `SimulatedBullet` with layout metadata.
pub async fn handle_generate(
    State(state): State<AppState>,
    Json(request): Json<GenerateRequest>,
) -> Result<Json<GenerateResponse>, AppError> {
    if request.jd_text.trim().is_empty() {
        return Err(AppError::Validation("jd_text cannot be empty".to_string()));
    }

    let response = generate_resume(
        &state.db,
        &state.llm,
        state.fit_scorer.as_ref(),
        &state.page_config,
        Some(&state.redis),
        true, // grounding_enabled: Phase 5 — real grounding scores
        request,
        &state.config,
    )
    .await?;

    Ok(Json(GenerateResponse {
        resume_id: response.resume_id,
        fit_report: response.fit_report,
        bullets: response.bullets,
        status: response.status,
        layout_flagged: response.layout_flagged,
    }))
}

// ────────────────────────────────────────────────────────────────────────────
// Cached fit score lookup types + handler
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct CachedFitScoreRequest {
    pub user_id: Uuid,
    pub jd_text: String,
}

#[derive(Debug, Serialize)]
pub struct CachedFitScoreResponse {
    pub fit_report: FitReport,
    /// Always `true` — this endpoint is a cache-only lookup.
    pub cache_hit: bool,
    pub jd_hash: String,
    pub context_hash: String,
}

/// POST /api/v1/resumes/fit-score/cached
///
/// Cache-only fit score lookup — never calls the LLM.
/// Returns `404` if no cached score exists for the given (user, jd, context) triple.
/// Used by the editor on page load to restore a previous fit score without incurring LLM cost.
pub async fn handle_get_cached_fit_score(
    State(state): State<AppState>,
    Json(request): Json<CachedFitScoreRequest>,
) -> Result<Json<CachedFitScoreResponse>, AppError> {
    if request.jd_text.trim().is_empty() {
        return Err(AppError::Validation("jd_text cannot be empty".to_string()));
    }

    let entries = get_current_entries(&state.db, request.user_id)
        .await
        .map_err(AppError::Internal)?;

    let jd_hash = hash_utils::compute_jd_hash(&request.jd_text);
    let context_hash = hash_utils::compute_context_hash(&entries);

    match fit_cache::lookup_cache(&state.db, request.user_id, &jd_hash, &context_hash).await {
        Ok(Some(fit_report)) => {
            tracing::debug!(
                user_id = %request.user_id,
                jd_hash = %jd_hash,
                context_hash = %context_hash,
                "fit-score/cached hit"
            );
            Ok(Json(CachedFitScoreResponse {
                fit_report,
                cache_hit: true,
                jd_hash,
                context_hash,
            }))
        }
        Ok(None) => Err(AppError::NotFound("no cached fit score".to_string())),
        Err(e) => {
            tracing::warn!(error = %e, "fit-score/cached lookup failed");
            // Treat any DB error as a cache miss (404) — caller will silently skip.
            Err(AppError::NotFound("no cached fit score".to_string()))
        }
    }
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
        "SELECT * FROM resume_bullets WHERE resume_id = $1 ORDER BY section, order_idx, id",
    )
    .bind(resume_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(ResumeDetailResponse { resume, bullets }))
}
