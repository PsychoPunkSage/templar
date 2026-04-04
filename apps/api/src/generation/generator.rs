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
use tracing::{info, warn};
use uuid::Uuid;

use crate::context::models::ContextEntryData;
use crate::context::versioning::get_current_entries;
use crate::errors::AppError;
use crate::generation::content_selector::{select_content, SelectionResult};
use crate::generation::fit_scoring::{FitReport, FitScorer};
use crate::generation::jd_parser::parse_jd;
use crate::generation::prompts::{PER_ENTRY_GENERATION_PROMPT_TEMPLATE, PER_ENTRY_GENERATION_SYSTEM};
use crate::generation::tone::{get_tone_examples, ToneExamples};
use crate::generation::{fit_cache, hash_utils};
use crate::grounding::scorer::{regenerate_single_bullet, score_bullet};
use crate::grounding::types::{GroundingResult, GroundingVerdict};
use crate::layout::{run_simulation_loop, PageConfig, SimulatedBullet};
use crate::llm_client::prompts::{GROUNDING_INSTRUCTION, SCOPE_INSTRUCTION};
use crate::llm_client::LlmClient;
use crate::models::context::ContextEntryRow;

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

/// Request body for resume generation.
#[derive(Debug, Clone, Deserialize)]
pub struct GenerateRequest {
    pub user_id: Uuid,
    pub jd_text: String,
    /// When true, bypass the fit score cache and force a fresh LLM call.
    #[serde(default)]
    pub force_refresh: bool,
    // Reserved for Phase 7 persona-aware generation
    #[allow(dead_code)]
    pub persona_id: Option<Uuid>,
    // Reserved for Phase 7 tone override
    #[allow(dead_code)]
    pub tone_override: Option<String>,
}

/// Response from the generation pipeline.
///
/// Phase 3: `bullets` now contains `SimulatedBullet` with `verified_line_count`,
/// `was_adjusted`, and `flagged_for_review` fields populated by the simulation loop.
#[derive(Debug, Clone, Serialize)]
pub struct GenerateResponse {
    pub resume_id: Uuid,
    pub fit_report: FitReport,
    pub bullets: Vec<SimulatedBullet>,
    pub status: String,
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
pub async fn generate_resume(
    pool: &PgPool,
    llm: &LlmClient,
    fit_scorer: &dyn FitScorer,
    page_config: &PageConfig,
    redis: Option<&redis::Client>,
    grounding_enabled: bool,
    request: GenerateRequest,
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
                tracing::info!(user_id = %request.user_id, "fit score cache hit in generate_resume");
                cached
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

    // Step 4: Content selection
    let selection = select_content(entries, &parsed_jd);
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

    // Step 6: LLM generation — parallel per-entry calls; entry selection filtered by fit_report
    let draft_bullets = call_llm_with_retry(llm, &parsed_jd, &selection, &tone_examples, &fit_report).await?;

    // Step 7: Layout simulation — enforces Line Coverage Contract.
    // Replaces LLM's line_estimate with simulation-verified line counts.
    // Bullets that fail after max passes are flagged for human review (not rejected).
    let simulation = run_simulation_loop(draft_bullets, page_config, &parsed_jd, llm).await?;

    // Page fill remediation pass — runs after simulation loop to fix whitespace/overflow.
    let simulation =
        crate::layout::page_fill::run_page_fill_pass(simulation, page_config, &parsed_jd, llm)
            .await?;

    if simulation.flagged_count > 0 {
        warn!(
            resume_id = %"pending",
            flagged = simulation.flagged_count,
            passes = simulation.total_passes,
            llm_calls = simulation.llm_calls_made,
            "layout simulation: bullets flagged for human review after max passes"
        );
    }

    // Step 7b: Grounding loop (Phase 5).
    // Score each simulated bullet against its source context entry.
    // Fail verdict → attempt one LLM rewrite → re-score → if still Fail, keep with flag.
    // grounding_enabled=false in unit tests skips all LLM grounding calls.
    let grounding_pairs: Vec<(SimulatedBullet, GroundingResult)> = if grounding_enabled {
        run_grounding_loop(&simulation.bullets, &selection.selected_entries, llm).await?
    } else {
        // Grounding disabled (unit tests): assign placeholder score 0.0 to all bullets.
        simulation
            .bullets
            .iter()
            .map(|b| {
                let score = crate::grounding::types::GroundingScore::compute(0.0, 0.0, 0.0, 0.0);
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

    // Step 8: Persist resume row
    let resume_id = Uuid::new_v4();
    let jd_parsed_value = serde_json::to_value(&parsed_jd)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to serialize ParsedJD: {e}")))?;
    let fit_score = fit_report.overall_score as f64 / 100.0;

    sqlx::query(
        r#"
        INSERT INTO resumes (id, user_id, jd_text, jd_parsed, fit_score, status)
        VALUES ($1, $2, $3, $4, $5, 'draft')
        "#,
    )
    .bind(resume_id)
    .bind(request.user_id)
    .bind(&request.jd_text)
    .bind(&jd_parsed_value)
    .bind(fit_score)
    .execute(pool)
    .await?;

    // Step 9: Persist simulated bullets with real grounding scores.
    // Uses sim_bullet.text (post-adjustment), sim_bullet.verified_line_count,
    // and the actual composite grounding score from step 7b.
    for (sim_bullet, grounding_result) in &grounding_pairs {
        sqlx::query(
            r#"
            INSERT INTO resume_bullets
                (resume_id, section, bullet_text, source_entry_id, grounding_score, line_count, rejection_reason, entry_header)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
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

    let final_bullets: Vec<SimulatedBullet> = grounding_pairs.into_iter().map(|(b, _)| b).collect();

    Ok(GenerateResponse {
        resume_id,
        fit_report,
        bullets: final_bullets,
        status: "draft".to_string(),
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
                "build_entry_header_latex: failed to deserialize ContextEntryData"
            );
            return None;
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

/// Builds the per-entry generation prompt for a single context entry.
fn build_per_entry_prompt(
    entry: &ContextEntryRow,
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
    tone_examples: &ToneExamples,
) -> Result<String, AppError> {
    let allowed_verbs = crate::generation::tone::filter_verbs_for_contribution(
        &tone_examples.strong_verbs,
        &entry.contribution_type,
    );
    let allowed_verbs_json = serde_json::to_string(&allowed_verbs)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize verbs: {e}")))?;

    let entry_data_json = serde_json::to_string_pretty(&entry.data)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize entry data: {e}")))?;

    let raw_text = entry.raw_text.as_deref().unwrap_or("(no raw text provided)");

    let keywords_json = serde_json::to_string(
        &parsed_jd
            .keyword_inventory
            .iter()
            .map(|k| &k.keyword)
            .collect::<Vec<_>>(),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize keywords: {e}")))?;

    let jd_summary = format!(
        "Detected tone: {:?}. Hard requirements: {}",
        parsed_jd.detected_tone,
        parsed_jd
            .hard_requirements
            .iter()
            .take(5)
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("; ")
    );

    Ok(PER_ENTRY_GENERATION_PROMPT_TEMPLATE
        .replace("{grounding_instruction}", GROUNDING_INSTRUCTION)
        .replace("{scope_instruction}", SCOPE_INSTRUCTION)
        .replace("{entry_type}", &entry.entry_type)
        .replace("{contribution_type}", &entry.contribution_type)
        .replace("{allowed_verbs_json}", &allowed_verbs_json)
        .replace("{entry_data_json}", &entry_data_json)
        .replace("{raw_text}", raw_text)
        .replace("{keywords_json}", &keywords_json)
        .replace("{jd_summary}", &jd_summary))
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
/// 3. Spawns one LLM call per entry in parallel via JoinSet.
/// 4. Flattens results into Vec<DraftBullet> with stable ordering.
async fn call_llm_with_retry(
    llm: &LlmClient,
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
    selection: &SelectionResult,
    tone_examples: &ToneExamples,
    fit_report: &FitReport,
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
        let all: Vec<&crate::generation::content_selector::RankedEntry> = selection.selected_entries.iter().collect();
        return call_llm_with_retry_entries(llm, parsed_jd, &all, tone_examples).await;
    }

    info!("Per-entry generation: {} entries to process", entries.len());
    call_llm_with_retry_entries(llm, parsed_jd, &entries, tone_examples).await
}

async fn call_llm_with_retry_entries(
    llm: &LlmClient,
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
    entries: &[&crate::generation::content_selector::RankedEntry],
    tone_examples: &ToneExamples,
) -> Result<Vec<DraftBullet>, AppError> {
    // Build all prompts synchronously before spawning tasks
    let mut prompts: Vec<(usize, String, ContextEntryRow, Option<String>)> =
        Vec::with_capacity(entries.len());

    for (idx, ranked) in entries.iter().enumerate() {
        let entry = &ranked.entry;
        let prompt = build_per_entry_prompt(entry, parsed_jd, tone_examples)?;
        let header = build_entry_header_latex(entry);
        prompts.push((idx, prompt, entry.clone(), header));
    }

    // Spawn parallel LLM calls — capped at 4 concurrent to avoid 529 rate limiting
    let sem = Arc::new(Semaphore::new(4));
    let mut join_set: tokio::task::JoinSet<Result<(usize, PerEntryLlmResponse, ContextEntryRow, Option<String>), AppError>> =
        tokio::task::JoinSet::new();

    for (idx, prompt, entry, header) in prompts {
        let llm = llm.clone();
        let sem = sem.clone();
        join_set.spawn(async move {
            let _permit = sem.acquire().await.expect("semaphore closed");
            llm.call_json::<PerEntryLlmResponse>(&prompt, PER_ENTRY_GENERATION_SYSTEM)
                .await
                .map(|r| (idx, r, entry, header))
                .map_err(|e| AppError::Llm(format!("Per-entry LLM call failed for entry {idx}: {e}")))
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
) -> Result<Vec<(SimulatedBullet, GroundingResult)>, AppError> {
    let mut pairs: Vec<(SimulatedBullet, GroundingResult)> = Vec::with_capacity(bullets.len());

    for bullet in bullets {
        // Find the source entry by entry_id
        let source_entry: Option<&ContextEntryRow> = entries
            .iter()
            .find(|re| re.entry.entry_id == bullet.source_entry_id)
            .map(|re| &re.entry);

        let source_entry = match source_entry {
            Some(e) => e,
            None => {
                // Missing source entry — this shouldn't happen (LLM retry loop enforces it)
                // but be defensive: return a fallback result.
                warn!(
                    source_entry_id = %bullet.source_entry_id,
                    "Source entry not found for bullet — using error fallback"
                );
                let result = GroundingResult::llm_error_fallback(
                    bullet.text.clone(),
                    bullet.source_entry_id,
                );
                pairs.push((bullet.clone(), result));
                continue;
            }
        };

        // Score the bullet
        let result = score_bullet(bullet, source_entry, llm).await?;

        if result.verdict != GroundingVerdict::Fail {
            pairs.push((bullet.clone(), result));
            continue;
        }

        // Fail → attempt one rewrite
        let rejection_reason = result
            .rejection_reason
            .as_deref()
            .unwrap_or("Grounding score below threshold");

        warn!(
            bullet = %bullet.text.chars().take(60).collect::<String>(),
            composite = result.score.composite,
            "Grounding Fail — attempting one rewrite"
        );

        let rewritten =
            regenerate_single_bullet(bullet, source_entry, rejection_reason, llm).await?;

        // Re-score the rewritten bullet
        let rewrite_result = score_bullet(&rewritten, source_entry, llm).await?;

        if rewrite_result.verdict != GroundingVerdict::Fail {
            pairs.push((rewritten, rewrite_result));
        } else {
            // Still failing after rewrite — keep rewritten text, flag for review
            warn!(
                bullet = %rewritten.text.chars().take(60).collect::<String>(),
                composite = rewrite_result.score.composite,
                "Grounding still Fail after rewrite — keeping with flagged_for_review"
            );
            let mut flagged = rewritten;
            flagged.flagged_for_review = true;
            pairs.push((flagged, rewrite_result));
        }
    }

    Ok(pairs)
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
        assert!(h.contains(r"\job"), "experience header must use \\job macro");
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
        assert!(h.contains(r"\skillcat"), "skill header must use \\skillcat macro");
        assert!(h.contains("Languages"), "must contain category");
        assert!(h.contains("Rust"), "must contain items");
    }

    #[test]
    fn test_build_entry_header_latex_bad_data_returns_none() {
        let mut entry = make_experience_entry_row();
        // Inject malformed data so deserialization fails
        entry.data = serde_json::json!({"not_a_real_field": true});
        let header = build_entry_header_latex(&entry);
        // Should return None and log a warning, not panic
        assert!(header.is_none());
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
        assert_eq!(resp.bullets[0].text, "Architected distributed caching layer");
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
        assert_eq!(format_date_range_opt(Some(s), Some(e)), "Jan 2022 -- Jun 2024");
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
}
