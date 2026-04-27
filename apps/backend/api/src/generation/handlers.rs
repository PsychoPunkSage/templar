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
use crate::generation::generator::{DraftBullet, EntryGroup, GenerateRequest, GenerateResponse};
use crate::generation::hash_utils;
use crate::generation::jd_parser::{parse_jd, ParsedJD};
use crate::generation::prompts::{REFINE_BULLET_PROMPT_TEMPLATE, REFINE_BULLET_SYSTEM};
use crate::generation::worker::enqueue_generation_job;
use crate::grounding::scorer::score_bullet;
use crate::grounding::types::GroundingVerdict;
use crate::layout::run_simulation_loop;
use crate::models::resume::{GenerationJobRow, ResumeBulletRow, ResumeRow};
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

/// Response from POST /api/v1/resumes/generate (FIX-08).
/// Returns immediately with a job_id — the actual generation runs in the background.
/// The frontend polls GET /api/v1/generation/jobs/:id/status every 3 seconds.
#[derive(Debug, Serialize)]
pub struct GenerateJobResponse {
    pub job_id: Uuid,
    /// Always "queued" on a successful enqueue.
    pub status: String,
}

/// Response from GET /api/v1/generation/jobs/:id/status (FIX-08 + FIX-10).
///
/// - `status` = queued | processing | done | failed
/// - `entry_groups` is populated on status='done'; contains structured per-entry grouping (FIX-10)
/// - `fit_report` and `layout_flagged` are populated on status='done'
/// - `error` is populated on status='failed'
#[derive(Debug, Serialize)]
pub struct GenerationStatusResponse {
    pub job_id: Uuid,
    pub status: String,
    pub error: Option<String>,
    pub resume_id: Option<Uuid>,
    pub fit_report: Option<FitReport>,
    pub entry_groups: Option<Vec<EntryGroup>>,
    pub layout_flagged: Option<bool>,
    /// Number of pages in the generated document. Null until status='done'.
    pub page_count: Option<u8>,
}

#[derive(Debug, Serialize)]
pub struct ResumeDetailResponse {
    pub resume: ResumeRow,
    pub bullets: Vec<ResumeBulletRow>,
    /// Typed display headers for the frontend editor — populated for resumes generated
    /// post-migration 014. Null for legacy resumes; frontend falls back to
    /// bulletRowsToEntryGroups() which uses entry_header LaTeX as label.
    pub entry_groups: Option<Vec<EntryGroup>>,
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

/// POST /api/v1/resumes/generate  (FIX-08)
///
/// Enqueues an async generation job and returns immediately with { job_id, status: "queued" }.
/// The actual pipeline (JD parse → fit score → LLM generate → layout simulation → grounding →
/// persist) runs in the background generation worker.
///
/// The frontend polls GET /api/v1/generation/jobs/:id/status every 3 seconds until done/failed.
pub async fn handle_generate(
    State(state): State<AppState>,
    Json(request): Json<GenerateRequest>,
) -> Result<Json<GenerateJobResponse>, AppError> {
    if request.jd_text.trim().is_empty() {
        return Err(AppError::Validation("jd_text cannot be empty".to_string()));
    }

    let user_id = request.user_id;
    let job_id = Uuid::new_v4();

    let request_json = serde_json::to_value(&request)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to serialize request: {e}")))?;

    // Insert job row with status='queued'
    sqlx::query(
        "INSERT INTO generation_jobs (id, user_id, request, status) VALUES ($1, $2, $3, 'queued')",
    )
    .bind(job_id)
    .bind(user_id)
    .bind(request_json)
    .execute(&state.db)
    .await?;

    // Enqueue job_id to Redis via spawn_blocking (enqueue_generation_job is synchronous)
    let redis = state.redis.clone();
    tokio::task::spawn_blocking(move || {
        if let Err(e) = enqueue_generation_job(&redis, job_id) {
            tracing::warn!("Failed to enqueue generation job {job_id}: {e}");
        }
    });

    tracing::info!(job_id = %job_id, user_id = %user_id, "Generation job enqueued");

    Ok(Json(GenerateJobResponse {
        job_id,
        status: "queued".to_string(),
    }))
}

/// GET /api/v1/generation/jobs/:id/status  (FIX-08 + FIX-10)
///
/// Returns the current status of an async generation job.
/// When status='done': also returns entry_groups, fit_report, and layout_flagged
/// from the stored result JSONB so the frontend can populate its editor state immediately.
pub async fn handle_generation_status(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<GenerationStatusResponse>, AppError> {
    let row = sqlx::query_as::<_, GenerationJobRow>("SELECT * FROM generation_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Generation job {job_id} not found")))?;

    // For non-terminal states, return status only — no result data yet
    if row.status != "done" {
        return Ok(Json(GenerationStatusResponse {
            job_id,
            status: row.status,
            error: row.error,
            resume_id: row.resume_id,
            fit_report: None,
            entry_groups: None,
            layout_flagged: None,
            page_count: None,
        }));
    }

    // status='done': deserialize the stored GenerateResponse from result JSONB
    let result: Option<GenerateResponse> = row.result.and_then(|v| serde_json::from_value(v).ok());

    let (fit_report, entry_groups, layout_flagged, page_count) = match result {
        Some(r) => (
            Some(r.fit_report),
            Some(r.entry_groups),
            Some(r.layout_flagged),
            Some(r.page_count),
        ),
        None => {
            // result JSONB missing or malformed — return done status with null fields.
            // This is non-fatal: the frontend can still render from resume_bullets.
            tracing::warn!(job_id = %job_id, "Generation job done but result JSONB missing or invalid");
            (None, None, None, None)
        }
    };

    Ok(Json(GenerationStatusResponse {
        job_id,
        status: "done".to_string(),
        error: None,
        resume_id: row.resume_id,
        fit_report,
        entry_groups,
        layout_flagged,
        page_count,
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

    // Deserialize stored entry_groups (None for legacy resumes pre-migration 014).
    // Deserialization failure is treated as None so old resumes don't error.
    let entry_groups: Option<Vec<EntryGroup>> = resume
        .entry_groups
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    Ok(Json(ResumeDetailResponse {
        resume,
        bullets,
        entry_groups,
    }))
}

// ────────────────────────────────────────────────────────────────────────────
// Inline bullet refinement
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct RefineBulletRequest {
    /// Current bullet text — used to identify the bullet in the DB.
    pub bullet_text: String,
    /// Source context entry ID — scopes LLM context to this entry only.
    pub source_entry_id: Uuid,
    /// Section the bullet belongs to (e.g. "experience").
    pub section: String,
    /// Free-form user instruction (e.g. "make this more concise").
    pub instruction: String,
}

#[derive(Debug, Serialize)]
pub struct RefineBulletResponse {
    pub original_text: String,
    /// On rejection: same as original_text. On success: the new bullet text.
    pub refined_text: String,
    pub verified_line_count: u8,
    pub grounding_score: f32,
    /// "pass" | "flag_for_review" | "fail"
    pub verdict: String,
    /// true when grounding score < 0.65 — frontend should revert to original_text.
    pub was_rejected: bool,
    pub rejection_reason: Option<String>,
}

/// Internal LLM output type for the refinement call.
#[derive(serde::Deserialize)]
struct RefinedBulletOutput {
    text: String,
}

/// POST /api/v1/resumes/:resume_id/bullets/refine
///
/// Refines a single bullet in-place using the user's instruction.
/// Context is scoped to the bullet's source entry only — not the full user context.
///
/// Pipeline: LLM rewrite → layout simulation → grounding check → DB update.
/// On grounding failure (< 0.65): returns was_rejected=true, DB unchanged.
/// On success: updates resume_bullets + resumes.entry_groups, caller should re-render.
pub async fn handle_refine_bullet(
    State(state): State<AppState>,
    Path(resume_id): Path<Uuid>,
    Json(request): Json<RefineBulletRequest>,
) -> Result<Json<RefineBulletResponse>, AppError> {
    if request.instruction.trim().is_empty() {
        return Err(AppError::Validation(
            "instruction cannot be empty".to_string(),
        ));
    }
    if request.bullet_text.trim().is_empty() {
        return Err(AppError::Validation(
            "bullet_text cannot be empty".to_string(),
        ));
    }

    // Step 1: Load resume → get jd_text + template_id + user_id
    let resume = sqlx::query_as::<_, ResumeRow>("SELECT * FROM resumes WHERE id = $1")
        .bind(resume_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Resume {resume_id} not found")))?;

    // Step 2: Load source entry (scoped context — only this entry sent to LLM).
    // FIX: Sending only source_entry context for single-bullet refinement.
    // Future: can scope further to specific achievements within the entry
    // that were originally selected (tracked via selected_entry_ids on FitReport).
    let all_entries = get_current_entries(&state.db, resume.user_id)
        .await
        .map_err(AppError::Internal)?;
    let source_entry = all_entries
        .into_iter()
        .find(|e| e.entry_id == request.source_entry_id)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "Context entry {} not found for this user",
                request.source_entry_id
            ))
        })?;

    // Step 3: Get page_config from template (for accurate layout simulation)
    let page_config = {
        let cache = state.template_cache.read().await;
        resume
            .template_id
            .as_deref()
            .and_then(|id| cache.get(id).map(|t| t.metadata.page_config()))
            .unwrap_or_else(|| state.page_config.clone())
    };

    // Step 4: Parse JD to extract keywords for the refinement prompt
    let parsed_jd = parse_jd(&resume.jd_text, &state.llm).await?;
    let keywords: Vec<&str> = parsed_jd
        .keyword_inventory
        .iter()
        .map(|k| k.keyword.as_str())
        .collect();
    let keywords_json = serde_json::to_string(&keywords).unwrap_or_else(|_| "[]".to_string());

    // Step 5: Fetch existing bullet's entry_header_latex so we preserve it on update
    let existing = sqlx::query!(
        "SELECT entry_header FROM resume_bullets \
         WHERE resume_id = $1 AND source_entry_id = $2 AND bullet_text = $3 \
         LIMIT 1",
        resume_id,
        request.source_entry_id,
        request.bullet_text,
    )
    .fetch_optional(&state.db)
    .await?;
    let entry_header_latex = existing.and_then(|r| r.entry_header);

    // Step 6: LLM refinement call
    let entry_data_json =
        serde_json::to_string(&source_entry.data).unwrap_or_else(|_| "{}".to_string());
    let raw_text = source_entry.raw_text.as_deref().unwrap_or("");

    let prompt = REFINE_BULLET_PROMPT_TEMPLATE
        .replace("{entry_data_json}", &entry_data_json)
        .replace("{raw_text}", raw_text)
        .replace("{jd_keywords_json}", &keywords_json)
        .replace("{current_bullet}", &request.bullet_text)
        .replace("{instruction}", &request.instruction);

    let llm_output: RefinedBulletOutput = state
        .llm
        .call_json(&prompt, REFINE_BULLET_SYSTEM)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("LLM refinement call failed: {e}")))?;

    let refined_text = llm_output.text.trim().to_string();
    if refined_text.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "LLM returned empty refined bullet"
        )));
    }

    // Step 7: Layout simulation (single bullet — enforces 2-line contract)
    let draft = DraftBullet {
        text: refined_text.clone(),
        source_entry_id: request.source_entry_id,
        section: request.section.clone(),
        entry_header_latex: entry_header_latex.clone(),
        line_estimate: 1,
        jd_keywords_used: vec![],
    };
    let sim_result =
        run_simulation_loop(vec![draft], &page_config, &parsed_jd, &state.llm, 1).await?;
    let simulated = sim_result
        .bullets
        .into_iter()
        .next()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("Simulation produced no bullets")))?;

    // Step 8: Grounding check
    let grounding = score_bullet(
        &simulated,
        &source_entry,
        &state.llm,
        simulated.was_adjusted,
    )
    .await?;
    let verdict_str = match &grounding.verdict {
        GroundingVerdict::Pass => "pass",
        GroundingVerdict::FlagForReview => "flag_for_review",
        GroundingVerdict::Fail => "fail",
    };

    // Step 9: Reject on grounding Fail — DB unchanged
    if matches!(grounding.verdict, GroundingVerdict::Fail) {
        tracing::warn!(
            resume_id = %resume_id,
            source_entry_id = %request.source_entry_id,
            score = grounding.score.composite,
            "Refined bullet rejected — grounding score below threshold"
        );
        return Ok(Json(RefineBulletResponse {
            original_text: request.bullet_text.clone(),
            refined_text: request.bullet_text,
            verified_line_count: 1,
            grounding_score: grounding.score.composite,
            verdict: verdict_str.to_string(),
            was_rejected: true,
            rejection_reason: grounding.rejection_reason,
        }));
    }

    // Step 10: Update DB — resume_bullets row.
    // Match by (resume_id, source_entry_id, bullet_text). Use TRIM on both sides so minor
    // whitespace differences (trailing newline from LLM, etc.) don't silently miss the row.
    let rows_updated = sqlx::query!(
        "UPDATE resume_bullets \
         SET bullet_text = $1, line_count = $2, grounding_score = $3, is_user_edited = true \
         WHERE resume_id = $4 AND source_entry_id = $5 AND TRIM(bullet_text) = TRIM($6)",
        simulated.text,
        simulated.verified_line_count as i16,
        grounding.score.composite as f64,
        resume_id,
        request.source_entry_id,
        request.bullet_text,
    )
    .execute(&state.db)
    .await?
    .rows_affected();

    if rows_updated == 0 {
        tracing::warn!(
            resume_id = %resume_id,
            source_entry_id = %request.source_entry_id,
            bullet_text_len = request.bullet_text.len(),
            "refine_bullet: UPDATE matched 0 rows — bullet_text mismatch; \
             content_hash will still be cleared to force re-render"
        );
    }

    // Always invalidate the render content-hash so the next render job never
    // hits the cache and always compiles fresh from resume_bullets.
    // This is safe even if rows_updated == 0 — at worst we get an extra compile.
    let _ = sqlx::query!(
        "UPDATE resumes SET content_hash = NULL WHERE id = $1",
        resume_id,
    )
    .execute(&state.db)
    .await;

    // Step 11: Update resumes.entry_groups JSONB (replace bullet in-place)
    if let Some(eg_value) = &resume.entry_groups {
        if let Ok(mut entry_groups) = serde_json::from_value::<Vec<EntryGroup>>(eg_value.clone()) {
            for group in &mut entry_groups {
                if group.source_entry_id == request.source_entry_id {
                    for bullet in &mut group.bullets {
                        if bullet.text == request.bullet_text {
                            bullet.text = simulated.text.clone();
                            bullet.verified_line_count = simulated.verified_line_count;
                            bullet.was_adjusted = true;
                            bullet.flagged_for_review =
                                matches!(grounding.verdict, GroundingVerdict::FlagForReview);
                            break;
                        }
                    }
                    break;
                }
            }
            if let Ok(new_eg) = serde_json::to_value(&entry_groups) {
                let _ = sqlx::query!(
                    "UPDATE resumes SET entry_groups = $1 WHERE id = $2",
                    new_eg,
                    resume_id,
                )
                .execute(&state.db)
                .await;
            }
        }
    }

    tracing::info!(
        resume_id = %resume_id,
        source_entry_id = %request.source_entry_id,
        verdict = verdict_str,
        score = grounding.score.composite,
        "Bullet refined successfully"
    );

    Ok(Json(RefineBulletResponse {
        original_text: request.bullet_text,
        refined_text: simulated.text,
        verified_line_count: simulated.verified_line_count,
        grounding_score: grounding.score.composite,
        verdict: verdict_str.to_string(),
        was_rejected: false,
        rejection_reason: None,
    }))
}
