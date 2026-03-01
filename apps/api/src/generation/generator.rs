//! Resume Generation — orchestrates the full generation pipeline.
//!
//! Flow: parse_jd → get_current_entries → fit_score → select_content →
//!       tone calibration → LLM generate → persist to DB → return response.
//!
//! This produces DRAFT bullets. Bullets are not shown to the user until
//! grounding (Phase 5) and layout (Phase 3) passes complete.

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
use crate::llm_client::prompts::{GROUNDING_INSTRUCTION, SCOPE_INSTRUCTION};
use crate::llm_client::LlmClient;

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
#[derive(Debug, Clone, Serialize)]
pub struct GenerateResponse {
    pub resume_id: Uuid,
    pub fit_report: FitReport,
    pub draft_bullets: Vec<DraftBullet>,
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
/// 7. INSERT into resumes (status='draft')
/// 8. INSERT into resume_bullets (grounding_score=0.0 placeholder — filled in Phase 5)
pub async fn generate_resume(
    pool: &PgPool,
    llm: &LlmClient,
    fit_scorer: &dyn FitScorer,
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

    // Step 7: Persist resume row
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

    // Step 8: Persist bullets (grounding_score=0.0 — Phase 5 will fill this)
    for bullet in &draft_bullets {
        sqlx::query(
            r#"
            INSERT INTO resume_bullets
                (resume_id, section, bullet_text, source_entry_id, grounding_score, line_count)
            VALUES ($1, $2, $3, $4, 0.0, $5)
            "#,
        )
        .bind(resume_id)
        .bind(&bullet.section)
        .bind(&bullet.text)
        .bind(bullet.source_entry_id)
        .bind(bullet.line_estimate as i16)
        .execute(pool)
        .await?;
    }

    info!(
        "Generated resume {} with {} draft bullets for user {}",
        resume_id,
        draft_bullets.len(),
        request.user_id
    );

    Ok(GenerateResponse {
        resume_id,
        fit_report,
        draft_bullets,
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
}
