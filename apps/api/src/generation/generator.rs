//! Resume Generation — orchestrates the full generation pipeline.
//!
//! Flow: parse_jd → get_current_entries → fit_score → select_content →
//!       tone calibration → LLM generate → layout simulation → grounding → persist to DB → return response.
//!
//! Phase 3 inserts a simulation loop between LLM draft generation and DB persistence.
//! Bullets that fail the Line Coverage Contract are expanded or compressed (max 3 passes),
//! then flagged for human review if still violating.
//!
//! Phase 5 inserts a grounding loop between layout simulation and DB persistence.
//! Bullets with composite grounding score < 0.65 are regenerated once; if still failing,
//! kept with flagged_for_review=true. grounding_score is persisted with real values.

use std::sync::Arc;

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::context::models::ContextEntryData;
use crate::context::versioning::get_current_entries;
use crate::errors::AppError;
use crate::generation::content_selector::{select_content, SelectionResult};
use crate::generation::fit_scoring::{FitReport, FitScorer};
use crate::generation::jd_parser::parse_jd;
use crate::generation::prompts::{
    PER_ENTRY_GENERATION_PROMPT_TEMPLATE, PER_ENTRY_GENERATION_SYSTEM,
};
use crate::generation::tone::{get_tone_examples, ToneExamples};
use crate::generation::{fit_cache, hash_utils};
use crate::grounding::scorer::{regenerate_single_bullet, score_bullet};
use crate::grounding::types::{GroundingResult, GroundingScore, GroundingVerdict};
use crate::layout::{run_simulation_loop, PageConfig, SimulatedBullet};
use crate::llm_client::prompts::{GROUNDING_INSTRUCTION, SCOPE_INSTRUCTION};
use crate::llm_client::LlmClient;
use crate::models::context::ContextEntryRow;

type GenerateLlmTaskResult =
    Result<(usize, PerEntryLlmResponse, ContextEntryRow, Option<String>), AppError>;

// ────────────────────────────────────────────────────────────────────────────
// Data models
// ────────────────────────────────────────────────────────────────────────────

/// A single bullet within a DraftEntry (as returned by the LLM).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftEntryBullet {
    pub text: String,
    pub line_estimate: u8,
    #[serde(default)]
    pub jd_keywords_used: Vec<String>,
}

/// LLM response for a single context entry — bullets only, no header (header built by Rust).
#[derive(Debug, Clone, Deserialize)]
pub struct PerEntryLlmResponse {
    pub bullets: Vec<DraftEntryBullet>,
}

/// A single draft resume bullet produced by the generation LLM call.
///
/// CRITICAL: every bullet MUST carry `source_entry_id` — bullets without it are rejected.
/// `line_estimate` is the LLM's guess only — NOT trusted for layout (Phase 3 enforces).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftBullet {
    pub text: String,
    pub source_entry_id: Uuid,
    pub section: String,
    /// Pre-formatted LaTeX header using template macros (e.g. \job{...}).
    /// Non-None only for the first bullet of each source entry group.
    #[serde(default)]
    pub entry_header_latex: Option<String>,
    /// LLM estimate only — layout Phase 3 will re-simulate. Must be 1 or 2.
    pub line_estimate: u8,
    #[serde(default)]
    pub jd_keywords_used: Vec<String>,
}

/// Whether to generate a single-page resume or a multi-page CV.
///
/// `SinglePage` is the default and preserves all existing behaviour.
/// `Cv` lifts the per-page content limits and distributes bullets across pages
/// using the greedy entry-atomic paginator in `layout::paginator`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResumeMode {
    #[default]
    SinglePage,
    Cv,
}

/// Request body for resume generation.
/// Derives Serialize so the full payload can be stored as JSONB in generation_jobs.request,
/// allowing the background worker to reconstruct the request without the HTTP connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateRequest {
    pub user_id: Uuid,
    pub jd_text: String,
    /// When true, bypass the fit score cache and force a fresh LLM call.
    #[serde(default)]
    pub force_refresh: bool,
    /// Single-page resume (default) or multi-page CV.
    #[serde(default)]
    pub resume_mode: ResumeMode,
    /// Maximum number of pages for CV mode (ignored in SinglePage mode).
    #[serde(default = "GenerateRequest::default_max_pages")]
    pub max_pages: u8,
    // Reserved for Phase 7 persona-aware generation
    #[allow(dead_code)]
    pub persona_id: Option<Uuid>,
    // Reserved for Phase 7 tone override
    #[allow(dead_code)]
    pub tone_override: Option<String>,
}

impl GenerateRequest {
    fn default_max_pages() -> u8 {
        4
    }
}

/// Response from the generation pipeline.
///
/// Phase 3: `bullets` now contains `SimulatedBullet` with `verified_line_count`,
/// `was_adjusted`, and `flagged_for_review` fields populated by the simulation loop.
/// FIX-10: `entry_groups` provides structured entry-level grouping for the frontend editor.
/// Derives Deserialize so the worker can round-trip it through generation_jobs.result JSONB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GenerateResponse {
    pub resume_id: Uuid,
    pub fit_report: FitReport,
    pub bullets: Vec<SimulatedBullet>,
    pub status: String,
    /// True if the page fill pass could not resolve whitespace/overflow within MAX_FILL_PASSES.
    pub layout_flagged: bool,
    /// Bullets grouped by source context entry, with human-readable display headers.
    /// Populated by build_entry_groups() — empty only when generation produces 0 bullets.
    pub entry_groups: Vec<EntryGroup>,
    /// Number of pages in the generated document (1 for SinglePage, 1–max_pages for CV).
    #[serde(default = "GenerateResponse::default_page_count")]
    pub page_count: u8,
}

impl GenerateResponse {
    fn default_page_count() -> u8 {
        1
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Generation pipeline
// ────────────────────────────────────────────────────────────────────────────

/// Runs the full resume generation pipeline and persists results to the DB.
///
/// Steps:
/// 1. parse_jd() → ParsedJD
/// 2. get_current_entries() → Vec<ContextEntryRow>
/// 3. fit_scorer.score() → FitReport
/// 4. select_content() → SelectionResult
/// 5. tone calibration → ToneExamples
/// 6. LLM generate → Vec<DraftBullet> (retried if any bullet lacks source_entry_id)
/// 7. Layout simulation → Vec<SimulatedBullet> (Phase 3: enforces Line Coverage Contract)
///
/// 7b. Grounding loop (Phase 5): score each bullet; Fail → rewrite once; still Fail → flag
/// 8. INSERT into resumes (status='draft')
/// 9. INSERT into resume_bullets (grounding_score now real value, not 0.0 placeholder)
/// 10. Fire-and-forget render job enqueue (Phase 4; skipped when redis=None for tests)
///
/// `grounding_enabled` controls whether step 7b runs. Pass `true` in production,
/// `false` in unit tests to skip LLM grounding calls.
/// `config` provides concurrency tunables (generation_llm_concurrency).
#[allow(clippy::too_many_arguments)]
pub async fn generate_resume(
    pool: &PgPool,
    llm: &LlmClient,
    fit_scorer: &dyn FitScorer,
    page_config: &PageConfig,
    redis: Option<&redis::Client>,
    grounding_enabled: bool,
    request: GenerateRequest,
    config: &crate::config::Config,
) -> Result<GenerateResponse, AppError> {
    // Step 1: Parse JD
    info!("Parsing JD for user {}", request.user_id);
    let parsed_jd = parse_jd(&request.jd_text, llm).await?;
    info!("JD parsed: tone={:?}", parsed_jd.detected_tone);

    // Step 2: Load current context entries
    let entries = get_current_entries(pool, request.user_id)
        .await
        .map_err(AppError::Internal)?;

    if entries.is_empty() {
        return Err(AppError::Validation(
            "No context entries found. Add context before generating a resume.".to_string(),
        ));
    }

    // Step 3: Fit score (cache-aware)
    let jd_hash = hash_utils::compute_jd_hash(&request.jd_text);
    let context_hash = hash_utils::compute_context_hash(&entries);

    let fit_report = if !request.force_refresh {
        match fit_cache::lookup_cache(pool, request.user_id, &jd_hash, &context_hash).await {
            Ok(Some(cached)) => {
                if cached.selected_entry_ids.is_empty() {
                    // Cache predates selected_entry_ids (written before LlmFitScorer was wired in).
                    // Force a fresh score so entry filtering works correctly.
                    tracing::info!(user_id = %request.user_id, "fit score cache hit but selected_entry_ids empty — forcing fresh score");
                    let fresh = fit_scorer.score(&entries, &parsed_jd).await?;
                    if let Err(e) = fit_cache::upsert_cache(
                        pool,
                        request.user_id,
                        &jd_hash,
                        &context_hash,
                        &fresh,
                    )
                    .await
                    {
                        tracing::warn!(error = %e, "fit-score cache upsert failed after refresh (non-fatal)");
                    }
                    fresh
                } else {
                    tracing::info!(user_id = %request.user_id, "fit score cache hit in generate_resume");
                    cached
                }
            }
            Ok(None) => {
                let report = fit_scorer.score(&entries, &parsed_jd).await?;
                if let Err(e) =
                    fit_cache::upsert_cache(pool, request.user_id, &jd_hash, &context_hash, &report)
                        .await
                {
                    tracing::warn!(error = %e, "fit-score cache upsert failed (non-fatal)");
                }
                report
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "fit-score cache lookup failed, falling through to scorer"
                );
                fit_scorer.score(&entries, &parsed_jd).await?
            }
        }
    } else {
        let report = fit_scorer.score(&entries, &parsed_jd).await?;
        if let Err(e) =
            fit_cache::upsert_cache(pool, request.user_id, &jd_hash, &context_hash, &report).await
        {
            tracing::warn!(error = %e, "fit-score cache upsert failed (non-fatal)");
        }
        report
    };

    info!(
        "Fit score: {}/100 for user {}",
        fit_report.overall_score, request.user_id
    );

    // Step 4: Content selection (CV mode uses higher per-section limits)
    let selection = select_content(entries, &parsed_jd, request.resume_mode);
    info!(
        "Selected {} entries for generation",
        selection.selected_entries.len()
    );

    if selection.selected_entries.is_empty() {
        return Err(AppError::Validation(
            "No context entries passed selection. Ensure context entries have scores above threshold.".to_string(),
        ));
    }

    // Step 5: Tone calibration
    let tone_examples = get_tone_examples(&parsed_jd.detected_tone);

    // Step 5b: Separate skill entries — they bypass LLM generation, simulation, and grounding.
    // Skill headers are built in a dedicated pure-Rust phase after bullet generation.
    let (skill_ranked, non_skill_ranked): (Vec<_>, Vec<_>) = selection
        .selected_entries
        .iter()
        .partition(|re| re.entry.entry_type == "skill");

    let non_skill_selection = SelectionResult {
        selected_entries: non_skill_ranked.into_iter().cloned().collect(),
        excluded_entries: selection.excluded_entries.clone(),
        section_weights: selection.section_weights.clone(),
        reframe_hints: selection.reframe_hints.clone(),
    };

    // Step 6: LLM generation — parallel per-entry calls (non-skill entries only).
    let draft_bullets = call_llm_with_retry(
        llm,
        &parsed_jd,
        &non_skill_selection,
        &tone_examples,
        &fit_report,
        config.generation_llm_concurrency,
    )
    .await?;

    // Step 6b: Dedup pass — remove near-duplicate bullets (Jaccard > 0.75).
    let draft_bullets = dedup_bullets(draft_bullets);
    // Step 6b-2: Remove header-only placeholders for entries that produced zero bullets.
    // When the LLM returns {"bullets": []} for an entry, a header-only DraftBullet (text="")
    // is still emitted. Without this pass it would render as a floating bold label in the PDF.
    let draft_bullets = remove_dangling_headers(draft_bullets);

    // Step 6c: Skills phase — JD-filtered skill headers (pure Rust, no LLM).
    let skill_ranked_refs: Vec<&crate::generation::content_selector::RankedEntry> =
        skill_ranked.to_vec();
    debug!(
        skill_entry_count = skill_ranked_refs.len(),
        total_selected = selection.selected_entries.len(),
        "Skills phase: entries from selection"
    );
    let skill_draft_bullets = skills_phase(
        &draft_bullets,
        &skill_ranked_refs,
        &non_skill_selection.selected_entries,
        &parsed_jd,
    );
    info!(
        bullets = skill_draft_bullets.len(),
        input_entries = skill_ranked_refs.len(),
        "Skills phase complete"
    );
    if !skill_ranked_refs.is_empty() && skill_draft_bullets.is_empty() {
        warn!(
            input_entries = skill_ranked_refs.len(),
            "Skills phase: had skill entries but produced 0 bullets — check WARN logs above for deserialization failures"
        );
    }

    // Step 7: Layout simulation — enforces Line Coverage Contract (non-skill bullets only).
    // Skill headers have empty text and would falsely trigger TooShort violations.
    let simulation = run_simulation_loop(
        draft_bullets,
        page_config,
        &parsed_jd,
        llm,
        config.layout_llm_concurrency,
    )
    .await?;

    // Page fill remediation pass — runs after simulation loop to fix whitespace/overflow.
    // Single-page mode is always the "last page" for fill analysis purposes.
    let mut simulation =
        crate::layout::page_fill::run_page_fill_pass(simulation, page_config, &parsed_jd, llm, true)
            .await?;

    // CV mode: distribute bullets across pages using the greedy entry-atomic paginator.
    // For single-page mode, page_number stays 1 and page_count stays 1.
    if request.resume_mode == ResumeMode::Cv {
        // We need the entry_groups to pass to the paginator (for group-size lookup context).
        // Build a temporary group list from the current bullet set.
        // (The permanent entry_groups will be built later from grounding_pairs.)
        let temp_entry_groups = build_entry_groups_from_bullets(&simulation.bullets);
        let pagination = crate::layout::paginate_bullets(
            &simulation.bullets,
            &temp_entry_groups,
            page_config.usable_height_lines,
            request.max_pages,
        );
        crate::layout::apply_pagination(&mut simulation.bullets, &pagination);
        simulation.page_count = pagination.page_count;

        // Per-page page-fill pass: check each page independently (not the last page check).
        // For now, we do a single all-page check — individual page remediation is a Phase 7 item.
        // The page_fill for intermediate pages already skips TooMuchWhitespace (is_last_page=false).
        let actual_pages = pagination.page_count;
        if actual_pages > 1 {
            // Re-run page fill for just the final page's bullets.
            let last_page_bullets: Vec<crate::layout::SimulatedBullet> = simulation
                .bullets
                .iter()
                .filter(|b| b.page_number == actual_pages)
                .cloned()
                .collect();
            let last_page_sim = crate::layout::simulator::SimulationResult {
                bullets: last_page_bullets,
                total_passes: simulation.total_passes,
                violations_remaining: 0,
                flagged_count: 0,
                llm_calls_made: 0,
                tighten_spacing: false,
                page_fill_flagged: false,
                page_count: 1,
            };
            let last_page_result = crate::layout::page_fill::run_page_fill_pass(
                last_page_sim,
                page_config,
                &parsed_jd,
                llm,
                true, // is_last_page
            )
            .await?;
            // Merge back the last-page bullets (they may have been compressed/promoted)
            let mut last_page_idx = 0usize;
            for bullet in simulation.bullets.iter_mut() {
                if bullet.page_number == actual_pages {
                    if let Some(updated) = last_page_result.bullets.get(last_page_idx) {
                        *bullet = updated.clone();
                        bullet.page_number = actual_pages; // restore page number
                        last_page_idx += 1;
                    }
                }
            }
            if last_page_result.page_fill_flagged {
                simulation.page_fill_flagged = true;
            }
        }
    }

    if simulation.flagged_count > 0 {
        warn!(
            resume_id = %"pending",
            flagged = simulation.flagged_count,
            passes = simulation.total_passes,
            llm_calls = simulation.llm_calls_made,
            "layout simulation: bullets flagged for human review after max passes"
        );
    }

    // Step 7b: Grounding loop (Phase 5) — non-skill bullets only.
    // Skill headers are self-grounding (category + items directly from context).
    let content_grounding_pairs: Vec<(SimulatedBullet, GroundingResult)> = if grounding_enabled {
        run_grounding_loop(
            &simulation.bullets,
            &non_skill_selection.selected_entries,
            llm,
            config.grounding_llm_concurrency,
            &parsed_jd,
        )
        .await?
    } else {
        // Grounding disabled (unit tests): assign placeholder score 0.0 to all bullets.
        simulation
            .bullets
            .iter()
            .map(|b| {
                let score = GroundingScore::compute(0.0, 0.0, 0.0, 0.0);
                (
                    b.clone(),
                    GroundingResult {
                        bullet_text: b.text.clone(),
                        source_entry_id: b.source_entry_id,
                        score,
                        verdict: GroundingVerdict::FlagForReview,
                        rejection_reason: None,
                    },
                )
            })
            .collect()
    };

    // Step 7c: Convert skill header bullets to SimulatedBullet + Pass grounding result.
    let skill_pairs: Vec<(SimulatedBullet, GroundingResult)> = skill_draft_bullets
        .into_iter()
        .map(|b| {
            let sim = SimulatedBullet {
                text: b.text,
                source_entry_id: b.source_entry_id,
                section: b.section,
                entry_header_latex: b.entry_header_latex,
                verified_line_count: 1,
                jd_keywords_used: vec![],
                was_adjusted: false,
                flagged_for_review: false,
                page_number: 1,
            };
            let score = GroundingScore::compute(1.0, 1.0, 1.0, 0.0);
            let result = GroundingResult {
                bullet_text: sim.text.clone(),
                source_entry_id: sim.source_entry_id,
                score,
                verdict: GroundingVerdict::Pass,
                rejection_reason: None,
            };
            (sim, result)
        })
        .collect();

    // Merge: content bullets first (preserve section ordering), skill headers appended at end.
    let grounding_pairs: Vec<(SimulatedBullet, GroundingResult)> = content_grounding_pairs
        .into_iter()
        .chain(skill_pairs)
        .collect();

    // Step 8: Persist resume row
    let resume_id = Uuid::new_v4();
    let jd_parsed_value = serde_json::to_value(&parsed_jd)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to serialize ParsedJD: {e}")))?;
    let fit_score = fit_report.overall_score as f64 / 100.0;

    let resume_type_str = match request.resume_mode {
        ResumeMode::SinglePage => "single_page",
        ResumeMode::Cv => "cv",
    };
    let page_count_db = simulation.page_count as i16;

    sqlx::query(
        r#"
        INSERT INTO resumes (id, user_id, jd_text, jd_parsed, fit_score, status, resume_type, page_count)
        VALUES ($1, $2, $3, $4, $5, 'draft', $6, $7)
        "#,
    )
    .bind(resume_id)
    .bind(request.user_id)
    .bind(&request.jd_text)
    .bind(&jd_parsed_value)
    .bind(fit_score)
    .bind(resume_type_str)
    .bind(page_count_db)
    .execute(pool)
    .await?;

    // Step 9: Persist simulated bullets with real grounding scores.
    // Uses sim_bullet.text (post-adjustment), sim_bullet.verified_line_count,
    // and the actual composite grounding score from step 7b.
    // order_idx = rank_idx preserves the relevance-ranked order from SelectionResult.
    // This replaces ORDER BY source_entry_id (random UUID) in the render query.
    for (rank_idx, (sim_bullet, grounding_result)) in grounding_pairs.iter().enumerate() {
        sqlx::query(
            r#"
            INSERT INTO resume_bullets
                (resume_id, section, bullet_text, source_entry_id,
                 grounding_score, line_count, rejection_reason, entry_header, order_idx, page_number)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            "#,
        )
        .bind(resume_id)
        .bind(&sim_bullet.section)
        .bind(&sim_bullet.text)
        .bind(sim_bullet.source_entry_id)
        .bind(grounding_result.score.composite as f64)
        .bind(sim_bullet.verified_line_count as i16)
        .bind(grounding_result.rejection_reason.as_deref())
        .bind(sim_bullet.entry_header_latex.as_deref())
        .bind(rank_idx as i32)
        .bind(sim_bullet.page_number as i16)
        .execute(pool)
        .await?;
    }

    let grounding_pass_count = grounding_pairs
        .iter()
        .filter(|(_, r)| r.verdict == GroundingVerdict::Pass)
        .count();
    let grounding_fail_count = grounding_pairs
        .iter()
        .filter(|(_, r)| r.verdict == GroundingVerdict::Fail)
        .count();

    info!(
        "Generated resume {} with {} bullets (layout_passes={}, adjusted={}, layout_flagged={}, grounding_pass={}, grounding_fail={}) for user {}",
        resume_id,
        grounding_pairs.len(),
        simulation.total_passes,
        simulation.bullets.iter().filter(|b| b.was_adjusted).count(),
        simulation.flagged_count,
        grounding_pass_count,
        grounding_fail_count,
        request.user_id
    );

    // Step 10: Fire-and-forget render job enqueue (Phase 4).
    // Skipped when redis=None (existing tests without a Redis connection continue to work).
    if let Some(redis_client) = redis {
        let job_id = Uuid::new_v4();

        // Insert render_jobs row (queued)
        sqlx::query("INSERT INTO render_jobs (id, resume_id, status) VALUES ($1, $2, 'queued')")
            .bind(job_id)
            .bind(resume_id)
            .execute(pool)
            .await?;

        // Enqueue via spawn_blocking (enqueue_render_job_sync is a synchronous Redis call)
        let redis_for_enqueue = redis_client.clone();
        tokio::task::spawn_blocking(move || {
            if let Err(e) =
                crate::render::worker::enqueue_render_job_sync(&redis_for_enqueue, job_id)
            {
                warn!("Failed to enqueue render job {job_id} for resume {resume_id}: {e}");
            }
        });

        info!("Enqueued render job {} for resume {}", job_id, resume_id);
    }

    // FIX-10: Build structured entry groups for the frontend editor before consuming grounding_pairs.
    let entry_groups = build_entry_groups(&grounding_pairs, &non_skill_selection.selected_entries);

    // FIX-10: Persist entry_groups to resumes table for page-reload restoration.
    // Non-fatal on failure: the generation result is still valid; the page-reload path
    // falls back to bulletRowsToEntryGroups which uses entry_header LaTeX as label.
    if let Ok(eg_json) = serde_json::to_value(&entry_groups) {
        if let Err(e) = sqlx::query("UPDATE resumes SET entry_groups = $1 WHERE id = $2")
            .bind(&eg_json)
            .bind(resume_id)
            .execute(pool)
            .await
        {
            warn!(resume_id = %resume_id, error = %e,
                "Failed to persist entry_groups to resumes table (non-fatal)");
        }
    }

    let final_bullets: Vec<SimulatedBullet> = grounding_pairs.into_iter().map(|(b, _)| b).collect();

    Ok(GenerateResponse {
        resume_id,
        fit_report,
        bullets: final_bullets,
        status: "draft".to_string(),
        layout_flagged: simulation.page_fill_flagged,
        entry_groups,
        page_count: simulation.page_count,
    })
}

// ────────────────────────────────────────────────────────────────────────────
// LLM call with retry
// ────────────────────────────────────────────────────────────────────────────

/// Normalizes LLM-output section name to canonical capitalized form.
/// Accepts minor variants (case-insensitive) and maps to canonical names.
fn normalize_section(llm_section: &str) -> String {
    match llm_section.to_lowercase().trim() {
        "experience" => "Experience".to_string(),
        "project" | "projects" | "open_source" | "opensource" => "Projects".to_string(),
        "education" => "Education".to_string(),
        "skill" | "skills" => "Skills".to_string(),
        "publication" | "publications" => "Publications".to_string(),
        other => {
            // Trust LLM for custom sections; capitalize first letter
            let mut s = other.to_string();
            if let Some(c) = s.get_mut(0..1) {
                c.make_ascii_uppercase();
            }
            s
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Header building helpers
// ────────────────────────────────────────────────────────────────────────────

fn format_date(d: NaiveDate) -> String {
    d.format("%b %Y").to_string()
}

fn format_date_range_opt(start: Option<NaiveDate>, end: Option<NaiveDate>) -> String {
    match (start, end) {
        (Some(s), Some(e)) => format!("{} -- {}", format_date(s), format_date(e)),
        (Some(s), None) => format!("{} -- Present", format_date(s)),
        (None, Some(e)) => format!("-- {}", format_date(e)),
        (None, None) => String::new(),
    }
}

fn escape_header_latex(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str(r"\\"),
            '{' | '}' => {
                out.push('\\');
                out.push(ch);
            }
            '#' => out.push_str(r"\#"),
            '$' => out.push_str(r"\$"),
            '%' => out.push_str(r"\%"),
            '^' => out.push_str(r"\^{}"),
            '&' => out.push_str(r"\&"),
            '~' => out.push_str(r"\~{}"),
            '_' => out.push_str(r"\_"),
            other => out.push(other),
        }
    }
    out
}

/// Builds the LaTeX header macro string for a context entry from its typed data.
/// Returns None if the entry data cannot be deserialized (logs a warning).
fn build_entry_header_latex(entry: &ContextEntryRow) -> Option<String> {
    // Inject "entry_type" tag so serde can deserialize the tagged enum
    let mut data_with_tag = entry.data.clone();
    if let Some(obj) = data_with_tag.as_object_mut() {
        obj.insert(
            "entry_type".to_string(),
            Value::String(entry.entry_type.clone()),
        );
    }

    let entry_data: ContextEntryData = match serde_json::from_value(data_with_tag) {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!(
                entry_id = %entry.entry_id,
                entry_type = %entry.entry_type,
                error = %e,
                "build_entry_header_latex: deserialization failed — trying raw fallback"
            );
            return build_entry_header_latex_fallback(entry);
        }
    };

    Some(match entry_data {
        ContextEntryData::Experience(e) => {
            let end = e
                .date_end
                .map(format_date)
                .unwrap_or_else(|| "Present".to_string());
            format!(
                r#"\job{{{}}}{{{}}}{{{} -- {}}}"#,
                escape_header_latex(&e.company),
                escape_header_latex(&e.role),
                format_date(e.date_start),
                end
            )
        }
        ContextEntryData::Project(e) => {
            let tech = e.tech_stack.join(", ");
            let dates = format_date_range_opt(e.date_start, e.date_end);
            format!(
                r#"\project{{{}}}{{{}}}{{{}}}"#,
                escape_header_latex(&e.name),
                escape_header_latex(&tech),
                dates
            )
        }
        ContextEntryData::OpenSource(e) => {
            let tech = e.tech_stack.join(", ");
            format!(
                r#"\project{{{}}}{{{}}}{{}}"#,
                escape_header_latex(&e.project_name),
                escape_header_latex(&tech)
            )
        }
        ContextEntryData::Education(e) => {
            let end = e
                .date_end
                .map(format_date)
                .unwrap_or_else(|| "Present".to_string());
            let gpa_str = e.gpa.map(|g| format!("{:.2}", g)).unwrap_or_default();
            format!(
                r#"\education{{{} -- {}}}{{{}}}{{{}}}{{{}}}"#,
                format_date(e.date_start),
                end,
                escape_header_latex(&e.degree),
                escape_header_latex(&e.institution),
                gpa_str
            )
        }
        ContextEntryData::Skill(e) => {
            let items = e.items.join(", ");
            format!(
                r#"\skillcat{{{}}}{{{}}}"#,
                escape_header_latex(&e.category),
                escape_header_latex(&items)
            )
        }
        ContextEntryData::Award(e) => {
            format!(
                r#"\competition{{{}}}{{{}}}"#,
                escape_header_latex(&e.title),
                escape_header_latex(&e.issuer)
            )
        }
        ContextEntryData::Publication(e) => {
            format!(
                r#"\competition{{{}}}{{{}}}"#,
                escape_header_latex(&e.title),
                escape_header_latex(&e.venue)
            )
        }
        ContextEntryData::Extracurricular(e) => {
            format!(
                r#"\competition{{{}}}{{{}}}"#,
                escape_header_latex(&e.organization),
                escape_header_latex(&e.role)
            )
        }
        ContextEntryData::Certification(e) => {
            format!(
                r#"\competition{{{}}}{{{}}}"#,
                escape_header_latex(&e.name),
                escape_header_latex(&e.issuer)
            )
        }
    })
}

/// Raw-JSON fallback for building entry header LaTeX when typed deserialization fails.
/// Reads fields directly from `entry.data` (serde_json::Value) without requiring
/// a fully valid typed struct — tolerates missing/null fields gracefully.
fn build_entry_header_latex_fallback(entry: &ContextEntryRow) -> Option<String> {
    let data = &entry.data;
    match entry.entry_type.as_str() {
        "experience" => {
            let company = data.get("company").and_then(|v| v.as_str()).unwrap_or("");
            let role = data.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let start = data
                .get("date_start")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let end = data
                .get("date_end")
                .and_then(|v| v.as_str())
                .unwrap_or("Present");
            if company.is_empty() && role.is_empty() {
                return None;
            }
            Some(format!(
                r#"\job{{{}}}{{{}}}{{{} -- {}}}"#,
                escape_header_latex(company),
                escape_header_latex(role),
                start,
                end
            ))
        }
        "open_source" => {
            let name = data
                .get("project_name")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tech = data
                .get("tech_stack")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            if name.is_empty() {
                return None;
            }
            Some(format!(
                r#"\project{{{}}}{{{}}}{{}}"#,
                escape_header_latex(name),
                escape_header_latex(&tech)
            ))
        }
        "project" => {
            let name = data
                .get("name")
                .or_else(|| data.get("project_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tech = data
                .get("tech_stack")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let start = data
                .get("date_start")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let end = data.get("date_end").and_then(|v| v.as_str()).unwrap_or("");
            let dates = if start.is_empty() {
                String::new()
            } else if end.is_empty() {
                format!("{} -- Present", start)
            } else {
                format!("{} -- {}", start, end)
            };
            if name.is_empty() {
                return None;
            }
            Some(format!(
                r#"\project{{{}}}{{{}}}{{{}}}"#,
                escape_header_latex(name),
                escape_header_latex(&tech),
                dates
            ))
        }
        "education" => {
            let institution = data
                .get("institution")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let degree = data.get("degree").and_then(|v| v.as_str()).unwrap_or("");
            let start = data
                .get("date_start")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let end = data
                .get("date_end")
                .and_then(|v| v.as_str())
                .unwrap_or("Present");
            let gpa = data
                .get("gpa")
                .and_then(|v| v.as_f64())
                .map(|g| format!("{:.2}", g))
                .unwrap_or_default();
            if institution.is_empty() && degree.is_empty() {
                return None;
            }
            Some(format!(
                r#"\education{{{} -- {}}}{{{}}}{{{}}}{{{}}}"#,
                start,
                end,
                escape_header_latex(degree),
                escape_header_latex(institution),
                gpa
            ))
        }
        _ => None,
    }
}

/// Builds the per-entry generation prompt for a single context entry.
fn build_per_entry_prompt(
    entry: &ContextEntryRow,
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
    tone_examples: &ToneExamples,
    fit_report: &FitReport,
) -> Result<String, AppError> {
    let allowed_verbs = crate::generation::tone::filter_verbs_for_contribution(
        &tone_examples.strong_verbs,
        &entry.contribution_type,
    );
    let allowed_verbs_json = serde_json::to_string(&allowed_verbs)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize verbs: {e}")))?;

    // FIX-04: strip pre-written bullets from entry data to prevent LLM paraphrasing
    let mut data_for_prompt = entry.data.clone();
    if let Some(obj) = data_for_prompt.as_object_mut() {
        obj.remove("bullets");
    }
    let entry_data_json = serde_json::to_string_pretty(&data_for_prompt)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize entry data: {e}")))?;

    let raw_text = entry
        .raw_text
        .as_deref()
        .unwrap_or("(no raw text provided)");

    let keywords_json = serde_json::to_string(
        &parsed_jd
            .keyword_inventory
            .iter()
            .map(|k| &k.keyword)
            .collect::<Vec<_>>(),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize keywords: {e}")))?;

    // FIX-02: build full JD context (tone, seniority, all requirements, soft signals, top keywords)
    let hard_reqs_text = if parsed_jd.hard_requirements.is_empty() {
        "  none".to_string()
    } else {
        parsed_jd
            .hard_requirements
            .iter()
            .map(|r| {
                format!(
                    "  - [{}] {}",
                    if r.is_required {
                        "REQUIRED"
                    } else {
                        "preferred"
                    },
                    r.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let keywords_text = if parsed_jd.keyword_inventory.is_empty() {
        "  none".to_string()
    } else {
        parsed_jd
            .keyword_inventory
            .iter()
            .take(15)
            .map(|k| {
                format!(
                    "  - {} (freq={}, weight={:.1})",
                    k.keyword, k.frequency, k.position_weight
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let jd_context = format!(
        "TONE: {:?}  |  SENIORITY: {}  |  STARTUP: {}  |  IC_FOCUSED: {}\n\
         HARD REQUIREMENTS:\n{}\n\
         SOFT SIGNALS: {}\n\
         TOP KEYWORDS (by weight):\n{}",
        parsed_jd.detected_tone,
        parsed_jd.role_signals.seniority,
        parsed_jd.role_signals.is_startup,
        parsed_jd.role_signals.is_ic_focused,
        hard_reqs_text,
        if parsed_jd.soft_signals.is_empty() {
            "none".to_string()
        } else {
            parsed_jd.soft_signals.join("; ")
        },
        keywords_text,
    );

    // FIX-03: build per-entry fit context from the fit report
    let entry_fit_context = build_entry_fit_context(entry.entry_id, fit_report);

    Ok(PER_ENTRY_GENERATION_PROMPT_TEMPLATE
        .replace("{grounding_instruction}", GROUNDING_INSTRUCTION)
        .replace("{scope_instruction}", SCOPE_INSTRUCTION)
        .replace("{entry_type}", &entry.entry_type)
        .replace("{contribution_type}", &entry.contribution_type)
        .replace("{allowed_verbs_json}", &allowed_verbs_json)
        .replace("{entry_data_json}", &entry_data_json)
        .replace("{raw_text}", raw_text)
        .replace("{keywords_json}", &keywords_json)
        .replace("{jd_context}", &jd_context)
        .replace("{entry_fit_context}", &entry_fit_context))
}

/// Builds the JD fit context string for a single entry from the fit report.
///
/// Uses `selected_entry_ids` (UUID list set by LlmFitScorer) to determine whether
/// this entry was identified as relevant to the JD. If not selected, returns a
/// directive for the LLM to generate minimal content only.
///
/// NOTE: `FitMatch.context_evidence` is LLM-generated free-text (e.g. "5 years Rust
/// at Acme"), never a UUID, so we cannot filter matches per-entry. Instead we show
/// all overall strong/partial JD requirements as role context for every selected entry.
/// This is the correct semantic: every selected entry should address the JD requirements.
fn build_entry_fit_context(entry_id: uuid::Uuid, fit_report: &FitReport) -> String {
    // Gate: if the scorer returned entry IDs and this entry is not among them, tell the
    // LLM it was not selected — it should produce minimal/no bullets.
    let scorer_has_selection = !fit_report.selected_entry_ids.is_empty();
    let is_selected = fit_report.selected_entry_ids.contains(&entry_id);

    if scorer_has_selection && !is_selected {
        return "This entry was NOT selected as a strong JD match. \
                Only generate bullets if JD overlap is clearly evident from the entry data above. \
                Prefer returning an empty bullets list."
            .to_string();
    }

    // Show the JD requirements from the overall fit report as role context.
    let strong: Vec<&str> = fit_report
        .strong_matches
        .iter()
        .map(|m| m.jd_requirement.as_str())
        .collect();
    let partial: Vec<&str> = fit_report
        .partial_matches
        .iter()
        .map(|m| m.jd_requirement.as_str())
        .collect();

    format!(
        "Strong JD matches (user profile vs JD): {}\nPartial JD matches: {}",
        if strong.is_empty() {
            "none".to_string()
        } else {
            strong.join(", ")
        },
        if partial.is_empty() {
            "none".to_string()
        } else {
            partial.join(", ")
        },
    )
}

// ────────────────────────────────────────────────────────────────────────────
// Fix 4: Post-generation dedup pass
// ────────────────────────────────────────────────────────────────────────────

/// Removes near-duplicate bullets using pairwise Jaccard similarity on token sets.
///
/// When two bullets share > 75% of their tokens, the later one (lower entry rank) is dropped.
/// Empty-text bullets (skill headers) are never compared or removed.
fn dedup_bullets(bullets: Vec<DraftBullet>) -> Vec<DraftBullet> {
    use std::collections::HashSet;

    let tokenize = |text: &str| -> HashSet<String> {
        text.split(|c: char| !c.is_alphanumeric())
            .filter(|t| t.len() > 2)
            .map(|t| t.to_lowercase())
            .collect()
    };

    let n = bullets.len();
    let mut to_remove: HashSet<usize> = HashSet::new();

    for i in 0..n {
        if to_remove.contains(&i) || bullets[i].text.is_empty() {
            continue;
        }
        let tokens_i = tokenize(&bullets[i].text);
        for (j, bullet_j) in bullets.iter().enumerate().skip(i + 1) {
            if to_remove.contains(&j) || bullet_j.text.is_empty() {
                continue;
            }
            let tokens_j = tokenize(&bullet_j.text);
            let intersection = tokens_i.intersection(&tokens_j).count();
            let union_count = tokens_i.union(&tokens_j).count();
            if union_count == 0 {
                continue;
            }
            let jaccard = intersection as f32 / union_count as f32;
            if jaccard > 0.75 {
                to_remove.insert(j); // drop the later (lower-ranked) bullet
            }
        }
    }

    if !to_remove.is_empty() {
        info!(
            dropped = to_remove.len(),
            "dedup: removed near-duplicate bullets"
        );
    }

    bullets
        .into_iter()
        .enumerate()
        .filter(|(i, _)| !to_remove.contains(i))
        .map(|(_, b)| b)
        .collect()
}

/// Removes header-only DraftBullet placeholders for entries that produced zero content bullets.
/// A header-only bullet (text == "") with no sibling content bullets for its source_entry_id
/// would render as a floating company/project label with no bullets under it in the PDF.
fn remove_dangling_headers(bullets: Vec<DraftBullet>) -> Vec<DraftBullet> {
    use std::collections::HashSet;
    let entries_with_content: HashSet<uuid::Uuid> = bullets
        .iter()
        .filter(|b| !b.text.is_empty())
        .map(|b| b.source_entry_id)
        .collect();
    bullets
        .into_iter()
        .filter(|b| {
            // Keep all content bullets unconditionally.
            // Keep header-only bullets ONLY if the entry has at least one content bullet.
            !b.text.is_empty() || entries_with_content.contains(&b.source_entry_id)
        })
        .collect()
}

// ────────────────────────────────────────────────────────────────────────────
// Fix 3: Skills as a separate post-generation phase
// ────────────────────────────────────────────────────────────────────────────

/// Builds JD-filtered skill header bullets from user's Skill context entries.
///
/// Algorithm:
/// 1. Collect all tech tags mentioned across the already-generated (non-skill) bullets.
/// 2. Union with JD keyword inventory to form a relevance set.
/// 3. Determine priority skill categories from JD role signals / tone.
/// 4. Score each Skill entry by how many items overlap the relevance set.
/// 5. Sort by (priority, score) and return `\skillcat{Category}{Items}` header bullets.
///
/// Skill header bullets have empty `text` — they bypass simulation and grounding.
fn skills_phase(
    draft_bullets: &[DraftBullet],
    skill_entries: &[&crate::generation::content_selector::RankedEntry],
    non_skill_entries: &[crate::generation::content_selector::RankedEntry],
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
) -> Vec<DraftBullet> {
    use std::collections::HashSet;

    if skill_entries.is_empty() {
        debug!("skills_phase: no skill entries selected — returning empty");
        return vec![];
    }

    // 1. Collect tech tags from source entries of generated bullets.
    //    Look up by source_entry_id in non_skill_entries (the selection we used for LLM generation).
    let entry_map: std::collections::HashMap<Uuid, &ContextEntryRow> = non_skill_entries
        .iter()
        .map(|re| (re.entry.entry_id, &re.entry))
        .collect();

    let source_tech: HashSet<String> = draft_bullets
        .iter()
        .filter_map(|b| entry_map.get(&b.source_entry_id))
        .flat_map(|e| e.tags.iter().map(|t| t.to_lowercase()))
        .collect();

    // 2. JD keywords
    let jd_keywords: HashSet<String> = parsed_jd
        .keyword_inventory
        .iter()
        .map(|k| k.keyword.to_lowercase())
        .collect();

    // 3. Relevance set
    let relevance: HashSet<String> = source_tech.union(&jd_keywords).cloned().collect();

    // 4. Role-based category priority list (index = priority; lower = higher priority)
    let has_linux = jd_keywords.contains("linux") || jd_keywords.contains("kernel");
    let priority_categories: Vec<&str> = if parsed_jd.role_signals.is_research {
        vec!["Languages", "Tools", "Frameworks"]
    } else if parsed_jd.role_signals.is_ic_focused || has_linux {
        vec!["Languages", "Linux", "OpenSource"]
    } else if matches!(
        parsed_jd.detected_tone,
        crate::generation::jd_parser::JDTone::AggressiveStartup
    ) {
        vec!["Languages", "Frameworks", "DevTools", "OpenSource"]
    } else {
        vec!["Languages", "Frameworks", "Databases", "DevTools"]
    };

    let priority_index = |cat: &str| -> usize {
        priority_categories
            .iter()
            .position(|&c| c.eq_ignore_ascii_case(cat))
            .unwrap_or(priority_categories.len()) // unlisted = lowest priority
    };

    // 5. Score and sort skill entries
    #[derive(Debug)]
    struct ScoredSkill<'a> {
        ranked: &'a crate::generation::content_selector::RankedEntry,
        category: String,
        filtered_items: Vec<String>,
        priority: usize,
        score: usize,
    }

    let mut scored: Vec<ScoredSkill> = skill_entries
        .iter()
        .filter_map(|re| {
            let mut data_with_tag = re.entry.data.clone();
            if let Some(obj) = data_with_tag.as_object_mut() {
                obj.insert(
                    "entry_type".to_string(),
                    Value::String(re.entry.entry_type.clone()),
                );
            }
            match serde_json::from_value::<ContextEntryData>(data_with_tag) {
                Ok(ContextEntryData::Skill(skill)) => {
                    let relevant_items: Vec<String> = skill
                        .items
                        .iter()
                        .filter(|item| relevance.contains(&item.to_lowercase()))
                        .cloned()
                        .collect();
                    let score = relevant_items.len();
                    let priority = priority_index(&skill.category);
                    Some(ScoredSkill {
                        ranked: re,
                        category: skill.category.clone(),
                        filtered_items: if score > 0 { relevant_items } else { skill.items.clone() },
                        priority,
                        score,
                    })
                }
                Ok(_other_variant) => {
                    warn!(
                        entry_id = %re.entry.entry_id,
                        entry_type = %re.entry.entry_type,
                        "skills_phase: entry deserialized to unexpected variant (not Skill) — skipping"
                    );
                    None
                }
                Err(e) => {
                    warn!(
                        entry_id = %re.entry.entry_id,
                        entry_type = %re.entry.entry_type,
                        error = %e,
                        "skills_phase: failed to deserialize entry data as ContextEntryData — skipping"
                    );
                    None
                }
            }
        })
        .collect();

    // Sort: high-priority categories first, then by relevance score descending
    scored.sort_by(|a, b| a.priority.cmp(&b.priority).then(b.score.cmp(&a.score)));

    let scored_count = scored.len();
    let relevant_count = scored.iter().filter(|s| s.score > 0).count();
    debug!(
        input_entries = skill_entries.len(),
        deserialized_ok = scored_count,
        jd_relevant = relevant_count,
        "skills_phase: scoring summary"
    );
    if scored_count == 0 {
        warn!("skills_phase: all skill entries failed deserialization — 0 bullets produced");
        return vec![];
    }

    // Fallback: if nothing scored, take top 4 by priority
    let to_emit: Vec<&ScoredSkill> = if scored.iter().any(|s| s.score > 0) {
        scored.iter().filter(|s| s.score > 0).collect()
    } else {
        scored.iter().take(4).collect()
    };

    // 6. Build header-only DraftBullets
    to_emit
        .into_iter()
        .map(|s| {
            let items_str = s.filtered_items.join(", ");
            let header = format!(
                r#"\skillcat{{{}}}{{{}}}"#,
                escape_header_latex(&s.category),
                escape_header_latex(&items_str)
            );
            DraftBullet {
                text: String::new(),
                source_entry_id: s.ranked.entry.entry_id,
                section: "Skills".to_string(),
                entry_header_latex: Some(header),
                line_estimate: 1,
                jd_keywords_used: vec![],
            }
        })
        .collect()
}

/// Infers the resume section name for a context entry type.
fn section_for_entry_type(entry_type: &str) -> &'static str {
    match entry_type.to_lowercase().trim() {
        "experience" => "Experience",
        "project" | "open_source" => "Projects",
        "education" => "Education",
        "skill" => "Skills",
        "publication" => "Publications",
        _ => "Other",
    }
}

/// Runs parallel per-entry LLM generation calls.
///
/// 1. Filters `selection.selected_entries` to `fit_report.selected_entry_ids` if non-empty.
/// 2. Builds entry headers from typed data (no LLM needed for headers).
/// 3. Spawns one LLM call per entry in parallel via JoinSet, capped at `generation_llm_concurrency`.
/// 4. Flattens results into Vec<DraftBullet> with stable ordering.
async fn call_llm_with_retry(
    llm: &LlmClient,
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
    selection: &SelectionResult,
    tone_examples: &ToneExamples,
    fit_report: &FitReport,
    generation_llm_concurrency: usize,
) -> Result<Vec<DraftBullet>, AppError> {
    // Filter entries: if fit_report has selected_entry_ids, honour them; else use all
    let entries: Vec<&crate::generation::content_selector::RankedEntry> =
        if fit_report.selected_entry_ids.is_empty() {
            selection.selected_entries.iter().collect()
        } else {
            let id_set: std::collections::HashSet<Uuid> =
                fit_report.selected_entry_ids.iter().cloned().collect();
            selection
                .selected_entries
                .iter()
                .filter(|re| id_set.contains(&re.entry.entry_id))
                .collect()
        };

    if entries.is_empty() {
        // Fallback: use all selected entries (e.g. if LLM returned IDs not in selection)
        warn!("call_llm_with_retry: no entries after filtering by selected_entry_ids — using all selected entries");
        let all: Vec<&crate::generation::content_selector::RankedEntry> =
            selection.selected_entries.iter().collect();
        return call_llm_with_retry_entries(
            llm,
            parsed_jd,
            &all,
            tone_examples,
            fit_report,
            generation_llm_concurrency,
        )
        .await;
    }

    info!("Per-entry generation: {} entries to process", entries.len());
    call_llm_with_retry_entries(
        llm,
        parsed_jd,
        &entries,
        tone_examples,
        fit_report,
        generation_llm_concurrency,
    )
    .await
}

async fn call_llm_with_retry_entries(
    llm: &LlmClient,
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
    entries: &[&crate::generation::content_selector::RankedEntry],
    tone_examples: &ToneExamples,
    fit_report: &FitReport,
    generation_llm_concurrency: usize,
) -> Result<Vec<DraftBullet>, AppError> {
    // Build all prompts synchronously before spawning tasks
    let mut prompts: Vec<(usize, String, ContextEntryRow, Option<String>)> =
        Vec::with_capacity(entries.len());

    for (idx, ranked) in entries.iter().enumerate() {
        let entry = &ranked.entry;
        let prompt = build_per_entry_prompt(entry, parsed_jd, tone_examples, fit_report)?;
        let header = build_entry_header_latex(entry);
        prompts.push((idx, prompt, entry.clone(), header));
    }

    // Spawn parallel LLM calls — capped at generation_llm_concurrency to avoid 429 rate limiting
    let sem = Arc::new(Semaphore::new(generation_llm_concurrency));
    let mut join_set: tokio::task::JoinSet<GenerateLlmTaskResult> = tokio::task::JoinSet::new();

    for (idx, prompt, entry, header) in prompts {
        let llm = llm.clone();
        let sem = sem.clone();
        join_set.spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            llm.call_json::<PerEntryLlmResponse>(&prompt, PER_ENTRY_GENERATION_SYSTEM)
                .await
                .map(|r| (idx, r, entry, header))
                .map_err(|e| {
                    AppError::Llm(format!("Per-entry LLM call failed for entry {idx}: {e}"))
                })
        });
    }

    // Collect results
    let mut results: Vec<(usize, PerEntryLlmResponse, ContextEntryRow, Option<String>)> =
        Vec::new();
    let mut errors: Vec<String> = Vec::new();

    while let Some(res) = join_set.join_next().await {
        match res {
            Ok(Ok(tuple)) => results.push(tuple),
            Ok(Err(e)) => errors.push(e.to_string()),
            Err(join_err) => errors.push(format!("JoinSet error: {join_err}")),
        }
    }

    if !errors.is_empty() {
        return Err(AppError::Llm(format!(
            "Per-entry generation failed: {}",
            errors.join("; ")
        )));
    }

    // Sort by original index for stable ordering
    results.sort_by_key(|(idx, _, _, _)| *idx);

    // Flatten into Vec<DraftBullet>
    let mut flat: Vec<DraftBullet> = Vec::new();

    for (_, resp, entry, header) in results {
        let section = normalize_section(section_for_entry_type(&entry.entry_type));

        if resp.bullets.is_empty() {
            // Skills-type entry or irrelevant entry: emit header-only placeholder
            flat.push(DraftBullet {
                text: String::new(),
                source_entry_id: entry.entry_id,
                section,
                entry_header_latex: header,
                line_estimate: 1,
                jd_keywords_used: Vec::new(),
            });
        } else {
            for (i, b) in resp.bullets.into_iter().enumerate() {
                if b.line_estimate > 2 {
                    warn!(
                        "Bullet has line_estimate={} (max 2) — layout will compress",
                        b.line_estimate
                    );
                }
                flat.push(DraftBullet {
                    text: b.text,
                    source_entry_id: entry.entry_id,
                    section: section.clone(),
                    // header only on first bullet of each entry group
                    entry_header_latex: if i == 0 { header.clone() } else { None },
                    line_estimate: b.line_estimate,
                    jd_keywords_used: b.jd_keywords_used,
                });
            }
        }
    }

    Ok(flat)
}

// ────────────────────────────────────────────────────────────────────────────
// Grounding loop (Phase 5)
// ────────────────────────────────────────────────────────────────────────────

/// Runs the grounding evaluation loop for all simulated bullets.
///
/// For each bullet:
/// 1. Find its source entry in the selected entries list.
/// 2. Score via `score_bullet()` (scope check + LLM).
/// 3. If Fail: attempt one rewrite via `regenerate_single_bullet()`, then re-score.
/// 4. If still Fail after rewrite: keep original, mark as flagged.
///
/// Returns `(SimulatedBullet, GroundingResult)` pairs — one per input bullet.
/// The SimulatedBullet may have updated text if a rewrite improved the score.
async fn run_grounding_loop(
    bullets: &[SimulatedBullet],
    entries: &[crate::generation::content_selector::RankedEntry],
    llm: &LlmClient,
    grounding_llm_concurrency: usize,
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
) -> Result<Vec<(SimulatedBullet, GroundingResult)>, AppError> {
    // Build owned (bullet, source_entry) pairs — owned data required for spawning tasks.
    // Bullets without a source entry get an immediate fallback result.
    let mut owned_bullets: Vec<SimulatedBullet> = Vec::with_capacity(bullets.len());
    let mut owned_entries: Vec<Option<ContextEntryRow>> = Vec::with_capacity(bullets.len());

    for bullet in bullets {
        let source_entry = entries
            .iter()
            .find(|re| re.entry.entry_id == bullet.source_entry_id)
            .map(|re| re.entry.clone());
        if source_entry.is_none() {
            warn!(
                source_entry_id = %bullet.source_entry_id,
                "Source entry not found for bullet — using error fallback"
            );
        }
        owned_bullets.push(bullet.clone());
        owned_entries.push(source_entry);
    }

    // ── Phase A: Score all bullets concurrently ──────────────────────────────
    let sem = Arc::new(Semaphore::new(grounding_llm_concurrency));
    let mut join_set: tokio::task::JoinSet<(usize, GroundingResult)> = tokio::task::JoinSet::new();

    for (i, (bullet, entry_opt)) in owned_bullets.iter().zip(owned_entries.iter()).enumerate() {
        let bullet = bullet.clone();
        match entry_opt {
            None => {
                // No source entry — push fallback immediately without spawning.
                // We'll collect these below alongside scored results.
                let result = GroundingResult::llm_error_fallback(
                    bullet.text.clone(),
                    bullet.source_entry_id,
                );
                // Use a direct future resolution via spawn to keep indexing uniform.
                join_set.spawn(async move { (i, result) });
            }
            Some(entry) => {
                let entry = entry.clone();
                let llm = llm.clone();
                let sem = Arc::clone(&sem);
                let was_adjusted = bullet.was_adjusted;
                join_set.spawn(async move {
                    let _permit = sem.acquire().await.expect("semaphore closed");
                    let result = score_bullet(&bullet, &entry, &llm, was_adjusted)
                        .await
                        .unwrap_or_else(|_| {
                            GroundingResult::llm_error_fallback(
                                bullet.text.clone(),
                                bullet.source_entry_id,
                            )
                        });
                    (i, result)
                });
            }
        }
    }

    let mut scores: Vec<Option<GroundingResult>> = vec![None; owned_bullets.len()];
    while let Some(res) = join_set.join_next().await {
        match res {
            Ok((i, result)) => scores[i] = Some(result),
            Err(e) => warn!(error = %e, "Grounding JoinSet task panicked"),
        }
    }

    // ── Phase B: Concurrent rewrites for failures ────────────────────────────
    let failures: Vec<(usize, SimulatedBullet, ContextEntryRow, String)> = scores
        .iter()
        .enumerate()
        .filter_map(|(i, score_opt)| {
            let score = score_opt.as_ref()?;
            if score.verdict != GroundingVerdict::Fail {
                return None;
            }
            let entry = owned_entries[i].as_ref()?;
            let reason = score
                .rejection_reason
                .clone()
                .unwrap_or_else(|| "Grounding score below threshold".to_string());
            warn!(
                bullet = %owned_bullets[i].text.chars().take(60).collect::<String>(),
                composite = score.score.composite,
                "Grounding Fail — attempting one rewrite"
            );
            Some((i, owned_bullets[i].clone(), entry.clone(), reason))
        })
        .collect();

    if !failures.is_empty() {
        let sem = Arc::new(Semaphore::new(grounding_llm_concurrency));
        let mut rewrite_set: tokio::task::JoinSet<(usize, SimulatedBullet, GroundingResult)> =
            tokio::task::JoinSet::new();

        for (i, bullet, entry, reason) in failures {
            let llm = llm.clone();
            let sem = Arc::clone(&sem);
            let parsed_jd = parsed_jd.clone();
            rewrite_set.spawn(async move {
                let _permit = sem.acquire().await.expect("semaphore closed");
                let rewritten =
                    regenerate_single_bullet(&bullet, &entry, &reason, &parsed_jd, &llm)
                        .await
                        .unwrap_or(bullet);
                let rescore = score_bullet(&rewritten, &entry, &llm, true)
                    .await
                    .unwrap_or_else(|_| {
                        GroundingResult::llm_error_fallback(
                            rewritten.text.clone(),
                            rewritten.source_entry_id,
                        )
                    });
                (i, rewritten, rescore)
            });
        }

        while let Some(res) = rewrite_set.join_next().await {
            match res {
                Ok((i, rewritten, rescore)) => {
                    if rescore.verdict == GroundingVerdict::Fail {
                        warn!(
                            bullet = %rewritten.text.chars().take(60).collect::<String>(),
                            composite = rescore.score.composite,
                            "Grounding still Fail after rewrite — keeping with flagged_for_review"
                        );
                        owned_bullets[i] = {
                            let mut flagged = rewritten;
                            flagged.flagged_for_review = true;
                            flagged
                        };
                    } else {
                        owned_bullets[i] = rewritten;
                    }
                    scores[i] = Some(rescore);
                }
                Err(e) => warn!(error = %e, "Grounding rewrite JoinSet task panicked"),
            }
        }
    }

    // ── Phase C: Assemble final pairs in original order ──────────────────────
    let pairs = owned_bullets
        .into_iter()
        .zip(scores.into_iter())
        .map(|(bullet, score_opt)| {
            let score = score_opt.unwrap_or_else(|| {
                GroundingResult::llm_error_fallback(bullet.text.clone(), bullet.source_entry_id)
            });
            (bullet, score)
        })
        .collect();

    Ok(pairs)
}

// ────────────────────────────────────────────────────────────────────────────
// FIX-10: Entry groups — structured per-entry output for the frontend editor
// ────────────────────────────────────────────────────────────────────────────

/// Human-readable display fields for one context entry, serialized as a tagged
/// JSON object so the frontend can switch on `type` without parsing LaTeX.
///
/// Mirrors the entry types in `context::models::ContextEntryData` but uses raw
/// JSON field extraction (no typed deserialization) so it never fails on
/// partial/malformed context data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntryDisplayHeader {
    Experience {
        company: String,
        role: String,
        date_range: String,
    },
    Project {
        name: String,
        tech_stack: String,
        date_range: String,
    },
    OpenSource {
        project_name: String,
        tech_stack: String,
    },
    Education {
        institution: String,
        degree: String,
        date_range: String,
    },
    Skills {
        category: String,
    },
    Other {
        label: String,
    },
}

/// All content bullets for one context entry, with human-readable display fields
/// for the frontend editor header row (company/role/dates above the bullet list).
///
/// Skills entries are excluded — they have no content bullets and are rendered
/// directly from `entry_header_latex` by the LaTeX render pipeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntryGroup {
    pub source_entry_id: Uuid,
    pub section: String,
    /// Parsed display fields for rendering the entry header in the editor UI.
    pub display_header: EntryDisplayHeader,
    /// Pre-formatted LaTeX header macro (e.g. `\job{...}{...}{...}`).
    /// Passed through to the render pipeline unchanged.
    pub entry_header_latex: Option<String>,
    /// Content bullets for this entry (never empty — entries with 0 content bullets are excluded).
    pub bullets: Vec<SimulatedBullet>,
}

/// Extracts human-readable display fields from raw entry.data JSON.
/// Uses raw field access (not typed deserialization) so it tolerates partial data
/// and never panics on malformed dates or missing fields.
fn build_entry_display_header(entry: &ContextEntryRow) -> EntryDisplayHeader {
    let d = &entry.data;

    match entry.entry_type.as_str() {
        "experience" => {
            let company = d
                .get("company")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let role = d
                .get("role")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let start = d.get("date_start").and_then(|v| v.as_str()).unwrap_or("?");
            let end = d
                .get("date_end")
                .and_then(|v| v.as_str())
                .unwrap_or("Present");
            EntryDisplayHeader::Experience {
                company,
                role,
                date_range: format!("{start} – {end}"),
            }
        }
        "open_source" => {
            let name = d
                .get("project_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tech = d
                .get("tech_stack")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            EntryDisplayHeader::OpenSource {
                project_name: name,
                tech_stack: tech,
            }
        }
        "project" => {
            let name = d
                .get("name")
                .or_else(|| d.get("project_name"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tech = d
                .get("tech_stack")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|s| s.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .unwrap_or_default();
            let start = d.get("date_start").and_then(|v| v.as_str()).unwrap_or("");
            let end = d.get("date_end").and_then(|v| v.as_str()).unwrap_or("");
            let date_range = match (start.is_empty(), end.is_empty()) {
                (true, _) => String::new(),
                (false, true) => format!("{start} – Present"),
                (false, false) => format!("{start} – {end}"),
            };
            EntryDisplayHeader::Project {
                name,
                tech_stack: tech,
                date_range,
            }
        }
        "education" => {
            let institution = d
                .get("institution")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let degree = d
                .get("degree")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let start = d.get("date_start").and_then(|v| v.as_str()).unwrap_or("?");
            let end = d
                .get("date_end")
                .and_then(|v| v.as_str())
                .unwrap_or("Present");
            EntryDisplayHeader::Education {
                institution,
                degree,
                date_range: format!("{start} – {end}"),
            }
        }
        "skills" => {
            let category = d
                .get("category")
                .and_then(|v| v.as_str())
                .unwrap_or("Skills")
                .to_string();
            EntryDisplayHeader::Skills { category }
        }
        _ => EntryDisplayHeader::Other {
            label: entry.entry_type.clone(),
        },
    }
}

/// Groups SimulatedBullets by source_entry_id and attaches human-readable display headers.
///
/// Called after the grounding loop, before returning from generate_resume().
/// Skills entries are excluded — they are header-only (empty text) with no content bullets,
/// and are rendered directly from entry_header_latex by the LaTeX render pipeline.
///
/// The output preserves the original relevance-ranked insertion order from the grounding pairs.
/// Builds a minimal `Vec<EntryGroup>` directly from `SimulatedBullet`s, without needing
/// the full context entry metadata. Used by the CV paginator to compute entry group sizes
/// before the permanent `build_entry_groups` call (which needs grounding_pairs).
fn build_entry_groups_from_bullets(bullets: &[SimulatedBullet]) -> Vec<EntryGroup> {
    use std::collections::HashMap;

    let mut seen_order: Vec<(Uuid, String, Option<String>)> = Vec::new();
    let mut seen_ids: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut bullet_map: HashMap<Uuid, Vec<SimulatedBullet>> = HashMap::new();

    for b in bullets {
        let eid = b.source_entry_id;
        if seen_ids.insert(eid) {
            seen_order.push((eid, b.section.clone(), b.entry_header_latex.clone()));
        }
        if !b.text.is_empty() {
            bullet_map.entry(eid).or_default().push(b.clone());
        }
    }

    seen_order
        .into_iter()
        .filter_map(|(entry_id, section, header_latex)| {
            let bullets = bullet_map.remove(&entry_id).unwrap_or_default();
            if bullets.is_empty() {
                return None;
            }
            Some(EntryGroup {
                source_entry_id: entry_id,
                section,
                display_header: EntryDisplayHeader::Other {
                    label: entry_id.to_string(),
                },
                entry_header_latex: header_latex,
                bullets,
            })
        })
        .collect()
}

fn build_entry_groups(
    grounding_pairs: &[(SimulatedBullet, GroundingResult)],
    entries: &[crate::generation::content_selector::RankedEntry],
) -> Vec<EntryGroup> {
    use std::collections::HashMap;

    // Build lookup: entry_id → &ContextEntryRow (non-skill entries only)
    let entry_map: HashMap<Uuid, &ContextEntryRow> = entries
        .iter()
        .map(|re| (re.entry.entry_id, &re.entry))
        .collect();

    // First pass: walk pairs in order to capture (entry_id, section, header_latex)
    // for each entry's first occurrence, and accumulate content bullets per entry.
    let mut seen_order: Vec<(Uuid, String, Option<String>)> = Vec::new();
    let mut seen_ids: std::collections::HashSet<Uuid> = std::collections::HashSet::new();
    let mut bullet_map: HashMap<Uuid, Vec<SimulatedBullet>> = HashMap::new();

    for (sim_bullet, _grounding) in grounding_pairs {
        let eid = sim_bullet.source_entry_id;

        if sim_bullet.text.is_empty() {
            // Header-only placeholder (including skills) — record first occurrence for ordering
            if seen_ids.insert(eid) {
                seen_order.push((
                    eid,
                    sim_bullet.section.clone(),
                    sim_bullet.entry_header_latex.clone(),
                ));
            }
        } else {
            // Content bullet — add to bucket
            if seen_ids.insert(eid) {
                // First time we see this entry via a content bullet (no preceding header placeholder)
                seen_order.push((
                    eid,
                    sim_bullet.section.clone(),
                    sim_bullet.entry_header_latex.clone(),
                ));
            }
            bullet_map.entry(eid).or_default().push(sim_bullet.clone());
        }
    }

    // Second pass: build EntryGroup for each entry that has at least one content bullet.
    // Entries with 0 content bullets (dangling headers, skills) are skipped.
    seen_order
        .into_iter()
        .filter_map(|(entry_id, section, header_latex)| {
            let bullets = bullet_map.remove(&entry_id).unwrap_or_default();
            if bullets.is_empty() {
                return None; // skip header-only / skill entries
            }
            let display_header = entry_map
                .get(&entry_id)
                .map(|e| build_entry_display_header(e))
                .unwrap_or_else(|| EntryDisplayHeader::Other {
                    label: entry_id.to_string(),
                });

            Some(EntryGroup {
                source_entry_id: entry_id,
                section,
                display_header,
                entry_header_latex: header_latex,
                bullets,
            })
        })
        .collect()
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_draft_bullet_serializes_and_deserializes() {
        let id = Uuid::new_v4();
        let bullet = DraftBullet {
            text: "Architected distributed caching layer reducing p99 latency by 40%".to_string(),
            source_entry_id: id,
            section: "experience".to_string(),
            entry_header_latex: None,
            line_estimate: 1,
            jd_keywords_used: vec!["distributed".to_string(), "latency".to_string()],
        };

        let json = serde_json::to_string(&bullet).unwrap();
        let recovered: DraftBullet = serde_json::from_str(&json).unwrap();

        assert_eq!(recovered.text, bullet.text);
        assert_eq!(recovered.source_entry_id, id);
        assert_eq!(recovered.section, "experience");
        assert_eq!(recovered.line_estimate, 1);
    }

    #[test]
    fn test_draft_bullet_requires_source_entry_id_in_json() {
        // A bullet JSON without source_entry_id should fail deserialization
        let bad_json = r#"{
            "text": "Did something",
            "section": "experience",
            "line_estimate": 1,
            "jd_keywords_used": []
        }"#;
        let result: Result<DraftBullet, _> = serde_json::from_str(bad_json);
        assert!(
            result.is_err(),
            "DraftBullet without source_entry_id must fail deserialization"
        );
    }

    #[test]
    fn test_line_estimate_max_is_2_by_convention() {
        // line_estimate > 2 is flagged but not rejected at this layer
        // (layout Phase 3 enforces the hard constraint)
        let bullet = DraftBullet {
            text: "Test".to_string(),
            source_entry_id: Uuid::new_v4(),
            section: "experience".to_string(),
            entry_header_latex: None,
            line_estimate: 2,
            jd_keywords_used: vec![],
        };
        assert!(bullet.line_estimate <= 2);
    }

    #[test]
    fn test_generate_request_deserialization() {
        let json = serde_json::json!({
            "user_id": Uuid::new_v4(),
            "jd_text": "We need a Rust engineer who can architect systems.",
            "persona_id": null,
            "tone_override": null
        });
        let request: GenerateRequest = serde_json::from_value(json).unwrap();
        assert!(!request.jd_text.is_empty());
        assert!(request.persona_id.is_none());
    }

    /// Backward compatibility: old JSON without `resume_mode` must deserialize to SinglePage.
    /// This is critical for the worker — stored generation_jobs.request JSONB created
    /// before migration 016 must not fail to deserialize after the upgrade.
    #[test]
    fn test_generate_request_backward_compat_no_resume_mode() {
        let legacy_json = serde_json::json!({
            "user_id": "00000000-0000-0000-0000-000000000001",
            "jd_text": "Legacy request without resume_mode field.",
            "persona_id": null,
            "tone_override": null
        });
        let request: GenerateRequest = serde_json::from_value(legacy_json)
            .expect("Legacy GenerateRequest (no resume_mode) must deserialize successfully");

        // Default must be SinglePage — not Cv
        assert_eq!(
            request.resume_mode,
            ResumeMode::SinglePage,
            "Missing resume_mode must default to SinglePage for backward compat"
        );
        // max_pages must also have a sensible default
        assert!(
            request.max_pages >= 1 && request.max_pages <= 8,
            "max_pages default must be in [1, 8], got {}",
            request.max_pages
        );
    }

    /// Verify that resume_mode=cv round-trips through JSON correctly.
    #[test]
    fn test_generate_request_cv_mode_serde_round_trip() {
        let json = serde_json::json!({
            "user_id": "00000000-0000-0000-0000-000000000001",
            "jd_text": "CV mode request for a senior researcher role.",
            "resume_mode": "cv",
            "max_pages": 3
        });
        let request: GenerateRequest = serde_json::from_value(json).unwrap();
        assert_eq!(request.resume_mode, ResumeMode::Cv);
        assert_eq!(request.max_pages, 3);

        // Serialize back and verify round-trip
        let serialized = serde_json::to_value(&request).unwrap();
        assert_eq!(serialized["resume_mode"], "cv");
        assert_eq!(serialized["max_pages"], 3);
    }

    fn make_experience_entry_row() -> ContextEntryRow {
        use chrono::NaiveDate;
        ContextEntryRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            version: 1,
            entry_type: "experience".to_string(),
            data: serde_json::json!({
                "company": "Acme Corp",
                "role": "Backend Engineer",
                "date_start": "2022-01-01",
                "date_end": null,
                "team_size": 5,
                "tech_stack": ["Rust", "Kubernetes"],
                "contribution_type": "primary_contributor",
                "location": null,
                "bullets": []
            }),
            raw_text: Some("Led development of distributed caching layer.".to_string()),
            recency_score: 0.9,
            impact_score: 0.8,
            tags: vec!["rust".to_string()],
            flagged_evergreen: false,
            contribution_type: "primary_contributor".to_string(),
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: chrono::Utc::now(),
        }
    }

    fn make_skill_entry_row() -> ContextEntryRow {
        ContextEntryRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            version: 1,
            entry_type: "skill".to_string(),
            data: serde_json::json!({
                "category": "Languages",
                "items": ["Rust", "Go", "Python"],
                "proficiency": null
            }),
            raw_text: None,
            recency_score: 1.0,
            impact_score: 0.5,
            tags: vec!["rust".to_string(), "go".to_string()],
            flagged_evergreen: true,
            contribution_type: "sole_author".to_string(),
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_build_entry_header_latex_experience() {
        let entry = make_experience_entry_row();
        let header = build_entry_header_latex(&entry);
        assert!(header.is_some(), "experience entry must produce a header");
        let h = header.unwrap();
        assert!(
            h.contains(r"\job"),
            "experience header must use \\job macro"
        );
        assert!(h.contains("Acme Corp"), "must contain company name");
        assert!(h.contains("Backend Engineer"), "must contain role");
        assert!(h.contains("Jan 2022"), "must contain start date");
        assert!(h.contains("Present"), "open end date must show Present");
    }

    #[test]
    fn test_build_entry_header_latex_skill() {
        let entry = make_skill_entry_row();
        let header = build_entry_header_latex(&entry);
        assert!(header.is_some(), "skill entry must produce a header");
        let h = header.unwrap();
        assert!(
            h.contains(r"\skillcat"),
            "skill header must use \\skillcat macro"
        );
        assert!(h.contains("Languages"), "must contain category");
        assert!(h.contains("Rust"), "must contain items");
    }

    #[test]
    fn test_build_entry_header_latex_empty_data_returns_none() {
        let mut entry = make_experience_entry_row();
        // Completely empty data — typed deserialization fails, fallback also returns None
        // because company and role are both empty strings.
        entry.data = serde_json::json!({});
        let header = build_entry_header_latex(&entry);
        // Should return None (fallback also returns None for empty company+role), not panic
        assert!(header.is_none());
    }

    #[test]
    fn test_build_entry_header_fallback_experience_missing_date() {
        let entry = ContextEntryRow {
            id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_type: "experience".to_string(),
            raw_text: None,
            data: serde_json::json!({"company": "Acme Corp", "role": "Senior Engineer"}),
            // date_start is absent — would fail typed deserialization
            contribution_type: "primary_contributor".to_string(),
            tags: vec![],
            recency_score: 0.8,
            impact_score: 0.8,
            version: 1,
            flagged_evergreen: false,
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: chrono::Utc::now(),
        };

        let result = build_entry_header_latex_fallback(&entry);
        assert!(
            result.is_some(),
            "fallback should return Some for experience with company+role"
        );
        let latex = result.unwrap();
        assert!(
            latex.contains("Acme Corp"),
            "fallback must include company name"
        );
        assert!(
            latex.contains("Senior Engineer"),
            "fallback must include role"
        );
        assert!(
            latex.contains('?'),
            "fallback must use '?' for missing date_start"
        );
        assert!(
            latex.contains("Present"),
            "fallback must use 'Present' for missing date_end"
        );
    }

    #[test]
    fn test_build_entry_header_fallback_open_source() {
        let entry = ContextEntryRow {
            id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_type: "open_source".to_string(),
            raw_text: None,
            data: serde_json::json!({"project_name": "MyLib", "tech_stack": ["Rust", "tokio"]}),
            contribution_type: "primary_contributor".to_string(),
            tags: vec![],
            recency_score: 0.8,
            impact_score: 0.8,
            version: 1,
            flagged_evergreen: false,
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: chrono::Utc::now(),
        };

        let result = build_entry_header_latex_fallback(&entry);
        assert!(
            result.is_some(),
            "fallback should return Some for open_source with project_name"
        );
        let latex = result.unwrap();
        assert!(
            latex.contains("MyLib"),
            "fallback must include project name"
        );
        assert!(latex.contains("Rust"), "fallback must include tech stack");
    }

    #[test]
    fn test_build_entry_header_fallback_unknown_type_returns_none() {
        let entry = ContextEntryRow {
            id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_type: "unknown_custom_type".to_string(),
            raw_text: None,
            data: serde_json::json!({"foo": "bar"}),
            contribution_type: "primary_contributor".to_string(),
            tags: vec![],
            recency_score: 0.8,
            impact_score: 0.8,
            version: 1,
            flagged_evergreen: false,
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: chrono::Utc::now(),
        };

        let result = build_entry_header_latex_fallback(&entry);
        assert!(
            result.is_none(),
            "unknown entry_type should return None from fallback"
        );
    }

    #[test]
    fn test_per_entry_llm_response_deserialization() {
        let json = r#"{
            "bullets": [
                {"text": "Architected distributed caching layer", "line_estimate": 1, "jd_keywords_used": ["distributed"]},
                {"text": "Reduced p99 latency by 40%", "line_estimate": 1, "jd_keywords_used": []}
            ]
        }"#;
        let resp: PerEntryLlmResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.bullets.len(), 2);
        assert_eq!(
            resp.bullets[0].text,
            "Architected distributed caching layer"
        );
        assert_eq!(resp.bullets[0].line_estimate, 1);
        assert_eq!(resp.bullets[0].jd_keywords_used, vec!["distributed"]);
    }

    #[test]
    fn test_per_entry_llm_response_empty_bullets() {
        // Skills entries return empty bullets array
        let json = r#"{"bullets": []}"#;
        let resp: PerEntryLlmResponse = serde_json::from_str(json).unwrap();
        assert!(resp.bullets.is_empty());
    }

    #[test]
    fn test_escape_header_latex_special_chars() {
        assert_eq!(escape_header_latex("A&B"), r"A\&B");
        assert_eq!(escape_header_latex("A_B"), r"A\_B");
        assert_eq!(escape_header_latex("A$B"), r"A\$B");
        assert_eq!(escape_header_latex("A%B"), r"A\%B");
        assert_eq!(escape_header_latex("A#B"), r"A\#B");
        // Plain text unchanged
        assert_eq!(escape_header_latex("Acme Corp"), "Acme Corp");
    }

    #[test]
    fn test_format_date_range_opt() {
        use chrono::NaiveDate;
        let s = NaiveDate::from_ymd_opt(2022, 1, 1).unwrap();
        let e = NaiveDate::from_ymd_opt(2024, 6, 1).unwrap();
        assert_eq!(
            format_date_range_opt(Some(s), Some(e)),
            "Jan 2022 -- Jun 2024"
        );
        assert_eq!(format_date_range_opt(Some(s), None), "Jan 2022 -- Present");
        assert_eq!(format_date_range_opt(None, None), "");
    }

    #[test]
    fn test_section_for_entry_type() {
        assert_eq!(section_for_entry_type("experience"), "Experience");
        assert_eq!(section_for_entry_type("project"), "Projects");
        assert_eq!(section_for_entry_type("open_source"), "Projects");
        assert_eq!(section_for_entry_type("education"), "Education");
        assert_eq!(section_for_entry_type("skill"), "Skills");
        assert_eq!(section_for_entry_type("publication"), "Publications");
        assert_eq!(section_for_entry_type("award"), "Other");
    }

    #[test]
    fn test_grounding_enabled_false_skips_scoring() {
        // When grounding_enabled=false, grounding_score placeholder is 0.0
        // This test verifies the type/logic at the unit level (no actual DB call).
        // The generate_resume integration path with grounding_enabled=false is verified
        // in integration tests (requires DB). Here we verify the placeholder tuple shape.
        let bullet = SimulatedBullet {
            text: "Contributed to distributed caching layer".to_string(),
            source_entry_id: Uuid::new_v4(),
            section: "experience".to_string(),
            entry_header_latex: None,
            verified_line_count: 1,
            jd_keywords_used: vec!["distributed".to_string()],
            was_adjusted: false,
            flagged_for_review: false,
            page_number: 1,
        };
        // When grounding is disabled, composite = 0.0 and verdict = FlagForReview
        let score = crate::grounding::types::GroundingScore::compute(0.0, 0.0, 0.0, 0.0);
        assert_eq!(score.composite, 0.0);
        assert_eq!(
            score.verdict(),
            crate::grounding::types::GroundingVerdict::Fail
        );
        // Bullet remains unchanged
        assert_eq!(bullet.text, "Contributed to distributed caching layer");
    }

    // ── dedup_bullets ────────────────────────────────────────────────────────

    fn make_draft_bullet(text: &str, entry_id: Uuid) -> DraftBullet {
        DraftBullet {
            text: text.to_string(),
            source_entry_id: entry_id,
            section: "Experience".to_string(),
            entry_header_latex: None,
            line_estimate: 1,
            jd_keywords_used: vec![],
        }
    }

    #[test]
    fn test_dedup_removes_near_duplicate() {
        let id = Uuid::new_v4();
        let b1 = make_draft_bullet(
            "Architected distributed caching layer using Redis and Rust",
            id,
        );
        // Slight rewording — high Jaccard overlap
        let b2 = make_draft_bullet(
            "Architected distributed caching layer using Redis and Rust system",
            id,
        );
        let b3 = make_draft_bullet("Reduced p99 latency by 40% via query optimisation", id);

        let result = dedup_bullets(vec![b1, b2, b3]);
        assert_eq!(result.len(), 2, "near-duplicate should be removed");
        assert_eq!(
            result[0].text,
            "Architected distributed caching layer using Redis and Rust"
        );
        assert_eq!(
            result[1].text,
            "Reduced p99 latency by 40% via query optimisation"
        );
    }

    #[test]
    fn test_dedup_keeps_distinct_bullets() {
        let id = Uuid::new_v4();
        let b1 = make_draft_bullet("Architected distributed caching layer", id);
        let b2 = make_draft_bullet("Reduced database query latency by 40%", id);
        let b3 = make_draft_bullet("Shipped new user onboarding flow", id);

        let result = dedup_bullets(vec![b1, b2, b3]);
        assert_eq!(result.len(), 3, "distinct bullets should all be kept");
    }

    #[test]
    fn test_dedup_skips_empty_text_bullets() {
        let id = Uuid::new_v4();
        let skill_header = make_draft_bullet("", id);
        let b1 = make_draft_bullet("Architected distributed caching layer", id);

        let result = dedup_bullets(vec![skill_header, b1]);
        // Both kept: empty text is never compared
        assert_eq!(result.len(), 2);
    }

    // ── skills_phase ─────────────────────────────────────────────────────────

    fn make_ranked_entry(
        entry: ContextEntryRow,
    ) -> crate::generation::content_selector::RankedEntry {
        crate::generation::content_selector::RankedEntry {
            entry,
            combined_score: 0.8,
            jd_relevance: 0.7,
        }
    }

    fn make_jd_with_keywords(keywords: &[&str]) -> crate::generation::jd_parser::ParsedJD {
        use crate::generation::jd_parser::{
            JDTone, KeywordEntry, ParsedJD, Requirement, RoleSignals,
        };
        ParsedJD {
            hard_requirements: vec![],
            soft_signals: vec![],
            role_signals: RoleSignals {
                is_startup: false,
                is_ic_focused: false,
                is_research: false,
                seniority: "senior".to_string(),
            },
            keyword_inventory: keywords
                .iter()
                .map(|k| KeywordEntry {
                    keyword: k.to_string(),
                    frequency: 2,
                    position_weight: 0.8,
                    weighted_score: 1.6,
                })
                .collect(),
            detected_tone: JDTone::CollaborativeEnterprise,
        }
    }

    fn make_skill_ranked_entry(
        category: &str,
        items: &[&str],
    ) -> crate::generation::content_selector::RankedEntry {
        let entry_id = Uuid::new_v4();
        let entry = ContextEntryRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_id,
            version: 1,
            entry_type: "skill".to_string(),
            data: serde_json::json!({
                "category": category,
                "items": items,
                "proficiency": null
            }),
            raw_text: None,
            recency_score: 1.0,
            impact_score: 0.5,
            tags: items.iter().map(|i| i.to_lowercase()).collect(),
            flagged_evergreen: true,
            contribution_type: "sole_author".to_string(),
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: chrono::Utc::now(),
        };
        make_ranked_entry(entry)
    }

    #[test]
    fn test_skills_phase_filters_to_jd_relevant_items() {
        let lang_entry = make_skill_ranked_entry("Languages", &["Rust", "Go", "Python", "COBOL"]);
        let skill_refs = vec![&lang_entry];

        // JD only mentions Rust and Go
        let parsed_jd = make_jd_with_keywords(&["Rust", "Go"]);

        // No draft bullets (empty context), so source_tech = {}; relevance = JD keywords only
        let bullets = skills_phase(&[], &skill_refs, &[], &parsed_jd);

        assert_eq!(bullets.len(), 1);
        let header = bullets[0].entry_header_latex.as_deref().unwrap_or("");
        assert!(header.contains("Languages"), "should include category");
        assert!(header.contains("Rust"), "Rust is JD-relevant");
        assert!(header.contains("Go"), "Go is JD-relevant");
        assert!(!header.contains("COBOL"), "COBOL is not JD-relevant");
        assert!(
            bullets[0].text.is_empty(),
            "skill bullet text must be empty"
        );
        assert_eq!(bullets[0].section, "Skills");
    }

    #[test]
    fn test_skills_phase_fallback_keeps_all_items_when_no_match() {
        let lang_entry = make_skill_ranked_entry("Languages", &["COBOL", "Fortran"]);
        let skill_refs = vec![&lang_entry];

        // JD has no keywords that match the skill items
        let parsed_jd = make_jd_with_keywords(&["Rust", "Kubernetes"]);

        let bullets = skills_phase(&[], &skill_refs, &[], &parsed_jd);

        // Fallback: no match → keep all items (top 4)
        assert_eq!(bullets.len(), 1);
        let header = bullets[0].entry_header_latex.as_deref().unwrap_or("");
        assert!(header.contains("COBOL"), "fallback: all items kept");
        assert!(header.contains("Fortran"), "fallback: all items kept");
    }

    #[test]
    fn test_skills_phase_empty_when_no_skill_entries() {
        let parsed_jd = make_jd_with_keywords(&["Rust"]);
        let bullets = skills_phase(&[], &[], &[], &parsed_jd);
        assert!(bullets.is_empty(), "no skill entries → empty result");
    }

    // ── FIX-02 / FIX-03 / FIX-04 ────────────────────────────────────────────

    fn make_fit_report_empty() -> FitReport {
        FitReport {
            overall_score: 0,
            strong_matches: vec![],
            partial_matches: vec![],
            gaps: vec![],
            recommendation: String::new(),
            scorer_backend: "keyword".to_string(),
            selected_entry_ids: vec![],
        }
    }

    #[test]
    fn test_build_per_entry_prompt_contains_full_jd_context() {
        use crate::generation::jd_parser::{
            JDTone, KeywordEntry, ParsedJD, Requirement, RoleSignals,
        };
        use crate::generation::tone::get_tone_examples;

        let entry = ContextEntryRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            version: 1,
            entry_type: "experience".to_string(),
            data: serde_json::json!({
                "company": "Acme",
                "role": "Engineer",
                "date_start": "2020-01-01",
                "date_end": "2022-01-01"
            }),
            raw_text: Some("Led backend work".to_string()),
            recency_score: 0.8,
            impact_score: 0.8,
            tags: vec![],
            flagged_evergreen: false,
            contribution_type: "primary_contributor".to_string(),
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: chrono::Utc::now(),
        };

        let parsed_jd = ParsedJD {
            detected_tone: JDTone::AggressiveStartup,
            soft_signals: vec![
                "cross-functional".to_string(),
                "ownership mindset".to_string(),
            ],
            hard_requirements: (0..10)
                .map(|i| Requirement {
                    text: format!("requirement_{}", i),
                    is_required: true,
                })
                .collect(),
            keyword_inventory: vec![KeywordEntry {
                keyword: "rust".to_string(),
                frequency: 5,
                position_weight: 0.9,
                weighted_score: 4.5,
            }],
            role_signals: RoleSignals {
                is_startup: true,
                is_ic_focused: true,
                is_research: false,
                seniority: "senior".to_string(),
            },
        };

        let tone_examples = get_tone_examples(&JDTone::AggressiveStartup);
        let fit_report = make_fit_report_empty();

        let prompt =
            build_per_entry_prompt(&entry, &parsed_jd, &tone_examples, &fit_report).unwrap();

        assert!(
            prompt.contains("requirement_0"),
            "prompt must contain first requirement"
        );
        assert!(
            prompt.contains("requirement_9"),
            "prompt must contain 10th requirement"
        );
        assert!(
            prompt.contains("cross-functional"),
            "prompt must contain soft signal"
        );
        assert!(prompt.contains("rust"), "prompt must contain keyword");
        assert!(
            prompt.contains("FULL JD CONTEXT"),
            "prompt must use new placeholder name"
        );
    }

    #[test]
    fn test_entry_data_json_strips_bullets_field() {
        use crate::generation::jd_parser::{JDTone, ParsedJD, RoleSignals};
        use crate::generation::tone::get_tone_examples;

        let entry = ContextEntryRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            version: 1,
            entry_type: "experience".to_string(),
            data: serde_json::json!({
                "company": "Acme",
                "role": "Engineer",
                "date_start": "2020-01-01",
                "bullets": ["pre-written bullet 1", "pre-written bullet 2"]
            }),
            raw_text: Some("test".to_string()),
            recency_score: 0.8,
            impact_score: 0.8,
            tags: vec![],
            flagged_evergreen: false,
            contribution_type: "primary_contributor".to_string(),
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: chrono::Utc::now(),
        };

        let parsed_jd = ParsedJD {
            detected_tone: JDTone::AggressiveStartup,
            soft_signals: vec![],
            hard_requirements: vec![],
            keyword_inventory: vec![],
            role_signals: RoleSignals {
                is_startup: false,
                is_ic_focused: false,
                is_research: false,
                seniority: "senior".to_string(),
            },
        };
        let tone_examples = get_tone_examples(&JDTone::AggressiveStartup);
        let fit_report = make_fit_report_empty();

        let prompt =
            build_per_entry_prompt(&entry, &parsed_jd, &tone_examples, &fit_report).unwrap();

        assert!(
            !prompt.contains("pre-written bullet"),
            "prompt must not contain pre-written bullets"
        );
        // The entry_data section must not include the bullets array values.
        // Note: "bullets" appears in the prompt template schema example, but the raw
        // bullet content ("pre-written bullet 1", etc.) must not appear.
        assert!(
            !prompt.contains("pre-written bullet 1"),
            "first pre-written bullet must be stripped"
        );
        assert!(
            !prompt.contains("pre-written bullet 2"),
            "second pre-written bullet must be stripped"
        );
    }

    #[test]
    fn test_entry_fit_context_selected_entry_shows_requirements() {
        use crate::generation::fit_scoring::{FitMatch, Gap};

        let entry_id = Uuid::new_v4();

        // context_evidence is LLM free-text, NOT a UUID — selection gate uses selected_entry_ids
        let fit_report = FitReport {
            overall_score: 85,
            strong_matches: vec![FitMatch {
                dimension: "Rust".to_string(),
                context_evidence: "5 years Rust at Acme".to_string(), // free-text, not UUID
                jd_requirement: "5+ years Rust experience".to_string(),
                strength: 0.9,
            }],
            partial_matches: vec![],
            gaps: vec![],
            recommendation: String::new(),
            scorer_backend: "llm".to_string(),
            selected_entry_ids: vec![entry_id], // entry IS selected
        };

        let result = build_entry_fit_context(entry_id, &fit_report);
        assert!(
            result.contains("5+ years Rust experience"),
            "selected entry must see strong match requirements"
        );
        assert!(
            result.contains("Strong JD matches"),
            "must use 'Strong JD matches' label"
        );
    }

    #[test]
    fn test_entry_fit_context_not_selected_returns_directive() {
        use crate::generation::fit_scoring::{FitMatch, Gap};

        let entry_id = Uuid::new_v4();
        let other_id = Uuid::new_v4(); // different entry was selected, not this one

        let fit_report = FitReport {
            overall_score: 85,
            strong_matches: vec![FitMatch {
                dimension: "Rust".to_string(),
                context_evidence: "5 years Rust at Acme".to_string(),
                jd_requirement: "5+ years Rust experience".to_string(),
                strength: 0.9,
            }],
            partial_matches: vec![],
            gaps: vec![],
            recommendation: String::new(),
            scorer_backend: "llm".to_string(),
            selected_entry_ids: vec![other_id], // entry_id is NOT in this list
        };

        let result = build_entry_fit_context(entry_id, &fit_report);
        // Should return the "not selected" directive so LLM minimises output
        assert!(
            result.contains("NOT selected"),
            "non-selected entry must get the 'NOT selected' directive"
        );
    }

    #[test]
    fn test_entry_fit_context_no_scorer_selection_shows_all_matches() {
        // When selected_entry_ids is empty (scorer didn't populate it or errored),
        // fall through and show all strong/partial matches without filtering.
        let entry_id = Uuid::new_v4();
        let fit_report = make_fit_report_empty(); // selected_entry_ids = []

        let result = build_entry_fit_context(entry_id, &fit_report);
        assert!(
            result.contains("none"),
            "when no matches in empty report, should say 'none'"
        );
        assert!(
            !result.contains("NOT selected"),
            "empty selected_entry_ids must not trigger 'NOT selected' directive"
        );
    }

    // ── remove_dangling_headers ──────────────────────────────────────────────

    #[test]
    fn test_remove_dangling_headers_removes_zero_bullet_entry() {
        let entry_a = Uuid::new_v4();
        let bullets = vec![DraftBullet {
            text: String::new(), // header-only
            source_entry_id: entry_a,
            entry_header_latex: Some(r"\job{Acme}{Eng}{2020 -- 2022}".to_string()),
            section: "experience".to_string(),
            line_estimate: 1,
            jd_keywords_used: vec![],
        }];
        let result = remove_dangling_headers(bullets);
        assert!(
            result.is_empty(),
            "header-only entry with no content bullets should be removed"
        );
    }

    #[test]
    fn test_remove_dangling_headers_keeps_headers_with_siblings() {
        let entry_a = Uuid::new_v4();
        let bullets = vec![
            DraftBullet {
                text: String::new(), // header
                source_entry_id: entry_a,
                entry_header_latex: Some(r"\job{Acme}{Eng}{2020 -- 2022}".to_string()),
                section: "experience".to_string(),
                line_estimate: 1,
                jd_keywords_used: vec![],
            },
            DraftBullet {
                text: "Built distributed system".to_string(),
                source_entry_id: entry_a,
                entry_header_latex: None,
                section: "experience".to_string(),
                line_estimate: 1,
                jd_keywords_used: vec![],
            },
            DraftBullet {
                text: "Reduced latency by 40%".to_string(),
                source_entry_id: entry_a,
                entry_header_latex: None,
                section: "experience".to_string(),
                line_estimate: 1,
                jd_keywords_used: vec![],
            },
        ];
        let result = remove_dangling_headers(bullets);
        assert_eq!(
            result.len(),
            3,
            "all 3 bullets (header + 2 content) should be kept"
        );
    }

    #[test]
    fn test_remove_dangling_headers_mixed() {
        let entry_a = Uuid::new_v4();
        let entry_b = Uuid::new_v4();
        let bullets = vec![
            DraftBullet {
                text: String::new(), // header for A
                source_entry_id: entry_a,
                entry_header_latex: Some(r"\job{Acme}{Eng}{2020 -- 2022}".to_string()),
                section: "experience".to_string(),
                line_estimate: 1,
                jd_keywords_used: vec![],
            },
            DraftBullet {
                text: "Built API".to_string(),
                source_entry_id: entry_a,
                entry_header_latex: None,
                section: "experience".to_string(),
                line_estimate: 1,
                jd_keywords_used: vec![],
            },
            DraftBullet {
                text: String::new(), // header for B — no content bullets
                source_entry_id: entry_b,
                entry_header_latex: Some(r"\job{Irrelevant Co}{Intern}{2019 -- 2019}".to_string()),
                section: "experience".to_string(),
                line_estimate: 1,
                jd_keywords_used: vec![],
            },
        ];
        let result = remove_dangling_headers(bullets);
        assert_eq!(
            result.len(),
            2,
            "entry_a header + content kept; entry_b header removed"
        );
        assert!(
            result
                .iter()
                .any(|b| b.source_entry_id == entry_a && b.text.is_empty()),
            "entry_a header kept"
        );
        assert!(
            result
                .iter()
                .any(|b| b.source_entry_id == entry_a && !b.text.is_empty()),
            "entry_a content kept"
        );
        assert!(
            !result.iter().any(|b| b.source_entry_id == entry_b),
            "entry_b header removed"
        );
    }

    // ── build_entry_display_header tests ─────────────────────────────────────

    fn make_test_entry(
        entry_type: &str,
        data: serde_json::Value,
    ) -> crate::models::context::ContextEntryRow {
        crate::models::context::ContextEntryRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            version: 1,
            entry_type: entry_type.to_string(),
            data,
            raw_text: None,
            recency_score: 1.0,
            impact_score: 1.0,
            tags: vec![],
            flagged_evergreen: false,
            contribution_type: "sole_author".to_string(),
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: chrono::Utc::now(),
        }
    }

    fn make_test_ranked_entry(
        entry_id: Uuid,
        entry_type: &str,
        data: serde_json::Value,
    ) -> crate::generation::content_selector::RankedEntry {
        let mut entry = make_test_entry(entry_type, data);
        entry.entry_id = entry_id;
        crate::generation::content_selector::RankedEntry {
            entry,
            combined_score: 1.0,
            jd_relevance: 1.0,
        }
    }

    fn make_content_sim_pair(
        entry_id: Uuid,
        section: &str,
        text: &str,
    ) -> (SimulatedBullet, crate::grounding::GroundingResult) {
        let score = crate::grounding::GroundingScore::compute(0.9, 0.9, 0.9, 0.0);
        (
            SimulatedBullet {
                text: text.to_string(),
                source_entry_id: entry_id,
                section: section.to_string(),
                entry_header_latex: None,
                verified_line_count: 1,
                jd_keywords_used: vec![],
                was_adjusted: false,
                flagged_for_review: false,
                page_number: 1,
            },
            crate::grounding::GroundingResult {
                bullet_text: text.to_string(),
                source_entry_id: entry_id,
                score,
                verdict: crate::grounding::GroundingVerdict::Pass,
                rejection_reason: None,
            },
        )
    }

    fn make_header_sim_pair(
        entry_id: Uuid,
        section: &str,
    ) -> (SimulatedBullet, crate::grounding::GroundingResult) {
        let score = crate::grounding::GroundingScore::compute(1.0, 1.0, 1.0, 0.0);
        (
            SimulatedBullet {
                text: String::new(),
                source_entry_id: entry_id,
                section: section.to_string(),
                entry_header_latex: Some(r"\skills{Languages}".to_string()),
                verified_line_count: 1,
                jd_keywords_used: vec![],
                was_adjusted: false,
                flagged_for_review: false,
                page_number: 1,
            },
            crate::grounding::GroundingResult {
                bullet_text: String::new(),
                source_entry_id: entry_id,
                score,
                verdict: crate::grounding::GroundingVerdict::Pass,
                rejection_reason: None,
            },
        )
    }

    #[test]
    fn test_display_header_experience_full() {
        let entry = make_test_entry(
            "experience",
            serde_json::json!({
                "company": "Acme Corp",
                "role": "Senior Engineer",
                "date_start": "Jan 2022",
                "date_end": "Jun 2023"
            }),
        );
        let h = build_entry_display_header(&entry);
        if let EntryDisplayHeader::Experience {
            company,
            role,
            date_range,
        } = h
        {
            assert_eq!(company, "Acme Corp");
            assert_eq!(role, "Senior Engineer");
            assert_eq!(date_range, "Jan 2022 \u{2013} Jun 2023");
        } else {
            panic!("expected Experience variant");
        }
    }

    #[test]
    fn test_display_header_experience_missing_fields_use_defaults() {
        // No dates, no role — must not panic; uses "?" and "Present" defaults
        let entry = make_test_entry("experience", serde_json::json!({ "company": "Acme" }));
        let h = build_entry_display_header(&entry);
        if let EntryDisplayHeader::Experience {
            role, date_range, ..
        } = h
        {
            assert_eq!(role, "");
            assert!(
                date_range.contains('?'),
                "missing start should use '?': {date_range}"
            );
            assert!(
                date_range.contains("Present"),
                "missing end should use 'Present': {date_range}"
            );
        } else {
            panic!("expected Experience variant");
        }
    }

    #[test]
    fn test_display_header_project_with_dates() {
        let entry = make_test_entry(
            "project",
            serde_json::json!({
                "name": "TempDB",
                "tech_stack": ["Rust", "Redis"],
                "date_start": "Mar 2023",
                "date_end": "Dec 2023"
            }),
        );
        if let EntryDisplayHeader::Project {
            name,
            tech_stack,
            date_range,
        } = build_entry_display_header(&entry)
        {
            assert_eq!(name, "TempDB");
            assert_eq!(tech_stack, "Rust, Redis");
            assert_eq!(date_range, "Mar 2023 \u{2013} Dec 2023");
        } else {
            panic!("expected Project variant");
        }
    }

    #[test]
    fn test_display_header_project_no_dates() {
        let entry = make_test_entry(
            "project",
            serde_json::json!({
                "name": "TempDB",
                "tech_stack": ["Rust"]
            }),
        );
        if let EntryDisplayHeader::Project { date_range, .. } = build_entry_display_header(&entry) {
            assert!(
                date_range.is_empty(),
                "no start → empty date_range, got: {date_range}"
            );
        } else {
            panic!("expected Project variant");
        }
    }

    #[test]
    fn test_display_header_open_source() {
        let entry = make_test_entry(
            "open_source",
            serde_json::json!({
                "project_name": "tokio",
                "tech_stack": ["Rust", "async"]
            }),
        );
        if let EntryDisplayHeader::OpenSource {
            project_name,
            tech_stack,
        } = build_entry_display_header(&entry)
        {
            assert_eq!(project_name, "tokio");
            assert_eq!(tech_stack, "Rust, async");
        } else {
            panic!("expected OpenSource variant");
        }
    }

    #[test]
    fn test_display_header_education() {
        let entry = make_test_entry(
            "education",
            serde_json::json!({
                "institution": "MIT",
                "degree": "BSc CS",
                "date_start": "Sep 2018",
                "date_end": "Jun 2022"
            }),
        );
        if let EntryDisplayHeader::Education {
            institution,
            degree,
            date_range,
        } = build_entry_display_header(&entry)
        {
            assert_eq!(institution, "MIT");
            assert_eq!(degree, "BSc CS");
            assert_eq!(date_range, "Sep 2018 \u{2013} Jun 2022");
        } else {
            panic!("expected Education variant");
        }
    }

    #[test]
    fn test_display_header_skills() {
        let entry = make_test_entry("skills", serde_json::json!({ "category": "Languages" }));
        if let EntryDisplayHeader::Skills { category } = build_entry_display_header(&entry) {
            assert_eq!(category, "Languages");
        } else {
            panic!("expected Skills variant");
        }
    }

    #[test]
    fn test_display_header_unknown_type_falls_back_to_other() {
        let entry = make_test_entry("certification", serde_json::json!({ "name": "AWS SA" }));
        if let EntryDisplayHeader::Other { label } = build_entry_display_header(&entry) {
            assert_eq!(label, "certification");
        } else {
            panic!("expected Other fallback");
        }
    }

    // ── build_entry_groups tests ──────────────────────────────────────────────

    #[test]
    fn test_build_entry_groups_basic_grouping() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let entries = vec![
            make_test_ranked_entry(
                id1,
                "experience",
                serde_json::json!({ "company": "Co1", "role": "SWE" }),
            ),
            make_test_ranked_entry(
                id2,
                "experience",
                serde_json::json!({ "company": "Co2", "role": "SRE" }),
            ),
        ];
        let pairs = vec![
            make_content_sim_pair(id1, "Experience", "bullet A1"),
            make_content_sim_pair(id1, "Experience", "bullet A2"),
            make_content_sim_pair(id2, "Experience", "bullet B1"),
        ];
        let groups = build_entry_groups(&pairs, &entries);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].source_entry_id, id1);
        assert_eq!(groups[0].bullets.len(), 2);
        assert_eq!(groups[1].source_entry_id, id2);
        assert_eq!(groups[1].bullets.len(), 1);
    }

    #[test]
    fn test_build_entry_groups_skills_excluded() {
        // Skill header (empty text) has no content bullets → must not appear in output
        let id_exp = Uuid::new_v4();
        let id_skill = Uuid::new_v4();
        let entries = vec![make_test_ranked_entry(
            id_exp,
            "experience",
            serde_json::json!({ "company": "Co", "role": "Dev" }),
        )];
        let pairs = vec![
            make_content_sim_pair(id_exp, "Experience", "real bullet"),
            make_header_sim_pair(id_skill, "Skills"), // empty text — skills header
        ];
        let groups = build_entry_groups(&pairs, &entries);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].source_entry_id, id_exp);
    }

    #[test]
    fn test_build_entry_groups_preserves_relevance_order() {
        // id2 appears first in grounding_pairs (higher ranked) → must be first in output
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let entries = vec![
            make_test_ranked_entry(id1, "experience", serde_json::json!({ "company": "A" })),
            make_test_ranked_entry(id2, "experience", serde_json::json!({ "company": "B" })),
        ];
        let pairs = vec![
            make_content_sim_pair(id2, "Experience", "bullet from id2"),
            make_content_sim_pair(id1, "Experience", "bullet from id1"),
        ];
        let groups = build_entry_groups(&pairs, &entries);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].source_entry_id, id2);
        assert_eq!(groups[1].source_entry_id, id1);
    }

    #[test]
    fn test_build_entry_groups_dangling_header_removed() {
        // Header-only entry (all bullets rejected / not in pairs) → must be filtered
        let id = Uuid::new_v4();
        let entries = vec![make_test_ranked_entry(
            id,
            "experience",
            serde_json::json!({ "company": "Ghost" }),
        )];
        let pairs = vec![make_header_sim_pair(id, "Experience")]; // no content bullets
        let groups = build_entry_groups(&pairs, &entries);
        assert!(
            groups.is_empty(),
            "dangling header entry must be removed from output"
        );
    }

    #[test]
    fn test_build_entry_groups_display_header_correctly_typed() {
        // Verify that the display_header is correctly typed (Experience, not Other fallback)
        let id = Uuid::new_v4();
        let entries = vec![make_test_ranked_entry(
            id,
            "experience",
            serde_json::json!({
                "company": "Acme", "role": "Staff Eng", "date_start": "2020", "date_end": "2023"
            }),
        )];
        let pairs = vec![make_content_sim_pair(id, "Experience", "some bullet")];
        let groups = build_entry_groups(&pairs, &entries);
        assert_eq!(groups.len(), 1);
        assert!(
            matches!(
                groups[0].display_header,
                EntryDisplayHeader::Experience { .. }
            ),
            "expected Experience display header, got: {:?}",
            groups[0].display_header
        );
        if let EntryDisplayHeader::Experience { company, .. } = &groups[0].display_header {
            assert_eq!(company, "Acme");
        }
    }

    #[test]
    fn test_build_entry_groups_multiple_bullets_per_entry_all_included() {
        let id = Uuid::new_v4();
        let entries = vec![make_test_ranked_entry(
            id,
            "experience",
            serde_json::json!({ "company": "Co" }),
        )];
        let pairs = vec![
            make_content_sim_pair(id, "Experience", "bullet 1"),
            make_content_sim_pair(id, "Experience", "bullet 2"),
            make_content_sim_pair(id, "Experience", "bullet 3"),
        ];
        let groups = build_entry_groups(&pairs, &entries);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].bullets.len(), 3);
        assert_eq!(groups[0].bullets[0].text, "bullet 1");
        assert_eq!(groups[0].bullets[2].text, "bullet 3");
    }
}
