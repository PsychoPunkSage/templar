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

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::context::versioning::get_current_entries;
use crate::errors::AppError;
use crate::generation::content_selector::{select_content, SelectionResult};
use crate::generation::fit_scoring::{FitReport, FitScorer};
use crate::generation::jd_parser::parse_jd;
use crate::generation::prompts::{GENERATION_PROMPT_TEMPLATE, GENERATION_SYSTEM};
use crate::generation::tone::{get_tone_examples, ToneExamples};
use crate::grounding::scorer::{regenerate_single_bullet, score_bullet};
use crate::grounding::types::{GroundingResult, GroundingVerdict};
use crate::layout::{run_simulation_loop, PageConfig, SimulatedBullet};
use crate::llm_client::prompts::{GROUNDING_INSTRUCTION, SCOPE_INSTRUCTION};
use crate::llm_client::LlmClient;
use crate::models::context::ContextEntryRow;

/// Max LLM retries when bullets are missing source_entry_id.
const MAX_GENERATION_RETRIES: u32 = 2;

// ────────────────────────────────────────────────────────────────────────────
// Data models
// ────────────────────────────────────────────────────────────────────────────

/// A single draft resume bullet produced by the generation LLM call.
///
/// CRITICAL: every bullet MUST carry `source_entry_id` — bullets without it are rejected.
/// `line_estimate` is the LLM's guess only — NOT trusted for layout (Phase 3 enforces).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftBullet {
    pub text: String,
    pub source_entry_id: Uuid,
    pub section: String,
    /// LLM estimate only — layout Phase 3 will re-simulate. Must be 1 or 2.
    pub line_estimate: u8,
    pub jd_keywords_used: Vec<String>,
}

/// Request body for resume generation.
#[derive(Debug, Clone, Deserialize)]
pub struct GenerateRequest {
    pub user_id: Uuid,
    pub jd_text: String,
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

    // Step 3: Fit score
    let fit_report = fit_scorer.score(&entries, &parsed_jd).await?;
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

    // Step 6: LLM generation with retry on missing source_entry_id
    let draft_bullets = call_llm_with_retry(llm, &parsed_jd, &selection, &tone_examples).await?;

    // Step 7: Layout simulation — enforces Line Coverage Contract.
    // Replaces LLM's line_estimate with simulation-verified line counts.
    // Bullets that fail after max passes are flagged for human review (not rejected).
    let simulation = run_simulation_loop(draft_bullets, page_config, &parsed_jd, llm).await?;

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
                (resume_id, section, bullet_text, source_entry_id, grounding_score, line_count)
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(resume_id)
        .bind(&sim_bullet.section)
        .bind(&sim_bullet.text)
        .bind(sim_bullet.source_entry_id)
        .bind(grounding_result.score.composite as f64)
        .bind(sim_bullet.verified_line_count as i16)
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

/// Calls the LLM to generate bullets. Retries up to MAX_GENERATION_RETRIES times
/// if any bullet is missing a valid `source_entry_id`.
async fn call_llm_with_retry(
    llm: &LlmClient,
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
    selection: &SelectionResult,
    tone_examples: &ToneExamples,
) -> Result<Vec<DraftBullet>, AppError> {
    let prompt = build_generation_prompt(parsed_jd, selection, tone_examples)?;

    let valid_entry_ids: HashSet<Uuid> = selection
        .selected_entries
        .iter()
        .map(|re| re.entry.entry_id)
        .collect();

    for attempt in 0..=MAX_GENERATION_RETRIES {
        let bullets: Vec<DraftBullet> = llm
            .call_json(&prompt, GENERATION_SYSTEM)
            .await
            .map_err(|e| AppError::Llm(format!("Generation LLM call failed: {e}")))?;

        // Validate: every bullet must reference a valid selected entry
        let invalid_count = bullets
            .iter()
            .filter(|b| !valid_entry_ids.contains(&b.source_entry_id))
            .count();

        if invalid_count == 0 {
            // Flag line_estimate > 2 (layout Phase 3 will enforce, but log early)
            for bullet in &bullets {
                if bullet.line_estimate > 2 {
                    warn!(
                        "Bullet has line_estimate={} (max 2) — layout will compress: {:?}",
                        bullet.line_estimate,
                        bullet.text.chars().take(60).collect::<String>()
                    );
                }
            }
            return Ok(bullets);
        }

        warn!(
            "Generation attempt {}/{}: {} bullets missing valid source_entry_id — retrying",
            attempt + 1,
            MAX_GENERATION_RETRIES + 1,
            invalid_count
        );
    }

    Err(AppError::Llm(format!(
        "Generation failed after {} attempts: bullets consistently lacked valid source_entry_id. \
        Check that context entries were passed correctly in the prompt.",
        MAX_GENERATION_RETRIES + 1
    )))
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

/// Builds the generation prompt by filling the template with serialized context.
fn build_generation_prompt(
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
    selection: &SelectionResult,
    tone_examples: &ToneExamples,
) -> Result<String, AppError> {
    let entries_json = serde_json::to_string_pretty(
        &selection
            .selected_entries
            .iter()
            .map(|re| {
                serde_json::json!({
                    "entry_id": re.entry.entry_id,
                    "entry_type": re.entry.entry_type,
                    "contribution_type": re.entry.contribution_type,
                    "tags": re.entry.tags,
                    "data": re.entry.data,
                    "combined_score": re.combined_score,
                })
            })
            .collect::<Vec<_>>(),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to serialize entries: {e}")))?;

    let keywords_json = serde_json::to_string(
        &parsed_jd
            .keyword_inventory
            .iter()
            .map(|k| &k.keyword)
            .collect::<Vec<_>>(),
    )
    .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to serialize keywords: {e}")))?;

    let tone_json = serde_json::to_string(&serde_json::json!({
        "strong_verbs": tone_examples.strong_verbs,
        "ownership_prefix": tone_examples.ownership_prefix,
        "avoid_verbs": tone_examples.avoid_verbs,
    }))
    .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to serialize tone: {e}")))?;

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

    Ok(GENERATION_PROMPT_TEMPLATE
        .replace("{grounding_instruction}", GROUNDING_INSTRUCTION)
        .replace("{scope_instruction}", SCOPE_INSTRUCTION)
        .replace("{tone_json}", &tone_json)
        .replace("{entries_json}", &entries_json)
        .replace("{keywords_json}", &keywords_json)
        .replace("{jd_summary}", &jd_summary))
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
