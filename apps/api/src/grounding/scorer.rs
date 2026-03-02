#![allow(dead_code)]

//! Grounding scorer — evaluates each resume bullet against its source context entry.
//!
//! # Flow
//! 1. Fast scope inflation check via `check_scope_inflation()` — no LLM token cost.
//! 2. LLM scoring call → 4-component JSON response.
//! 3. Compute composite score → derive verdict.
//!
//! # Fail-safe
//! If the LLM call errors (network, parse), log a warning and return
//! `GroundingVerdict::FlagForReview` with composite = 0.70. Generation is never
//! blocked entirely due to a grounding scorer failure.

use serde::Deserialize;
use tracing::warn;
use uuid::Uuid;

use crate::errors::AppError;
use crate::grounding::prompts::{GROUNDING_SCORE_PROMPT_TEMPLATE, GROUNDING_SCORE_SYSTEM};
use crate::grounding::scope_check::check_scope_inflation;
use crate::grounding::types::{GroundingResult, GroundingScore, GroundingVerdict};
use crate::layout::SimulatedBullet;
use crate::llm_client::LlmClient;
use crate::models::context::ContextEntryRow;

// ─────────────────────────────────────────────────────────────────────────────
// Internal LLM response type
// ─────────────────────────────────────────────────────────────────────────────

/// The structured JSON returned by the grounding scorer LLM call.
#[derive(Debug, Deserialize)]
struct LlmScoreResponse {
    source_match: f32,
    specificity_fidelity: f32,
    scope_accuracy: f32,
    interpolation_risk: f32,
    rejection_reason: Option<String>,
}

/// Response from the rewrite LLM call.
#[derive(Debug, Deserialize)]
struct RewrittenBullet {
    text: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Scores a single bullet against its source context entry.
///
/// Steps:
/// 1. Fast scope inflation check — if flagged, return synthetic Fail immediately.
/// 2. Build and call the grounding scorer LLM prompt.
/// 3. Compute composite score and derive verdict.
///
/// Fail-safe: on any LLM error, returns `FlagForReview` with composite ≈ 0.70.
pub async fn score_bullet(
    bullet: &SimulatedBullet,
    source_entry: &ContextEntryRow,
    llm: &LlmClient,
) -> Result<GroundingResult, AppError> {
    // Step 1: fast pre-LLM scope inflation check
    if let Some(reason) = check_scope_inflation(&bullet.text, &source_entry.contribution_type) {
        warn!(
            bullet = %bullet.text.chars().take(60).collect::<String>(),
            contribution_type = %source_entry.contribution_type,
            "Scope inflation detected — returning synthetic Fail without LLM call"
        );
        return Ok(GroundingResult::scope_inflation_fail(
            bullet.text.clone(),
            bullet.source_entry_id,
            reason,
        ));
    }

    // Step 2: build prompt
    let entry_data_json = serde_json::to_string(&source_entry.data)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to serialize entry data: {e}")))?;

    let prompt = GROUNDING_SCORE_PROMPT_TEMPLATE
        .replace("{bullet_text}", &bullet.text)
        .replace("{entry_id}", &source_entry.entry_id.to_string())
        .replace("{contribution_type}", &source_entry.contribution_type)
        .replace("{entry_data_json}", &entry_data_json);

    // Step 3: LLM call with fail-safe
    let llm_response: LlmScoreResponse = match llm.call_json(&prompt, GROUNDING_SCORE_SYSTEM).await
    {
        Ok(r) => r,
        Err(e) => {
            warn!(
                bullet = %bullet.text.chars().take(60).collect::<String>(),
                error = %e,
                "Grounding scorer LLM call failed — returning FlagForReview fallback"
            );
            return Ok(GroundingResult::llm_error_fallback(
                bullet.text.clone(),
                bullet.source_entry_id,
            ));
        }
    };

    // Step 4: compute score and verdict
    let score = GroundingScore::compute(
        llm_response.source_match.clamp(0.0, 1.0),
        llm_response.specificity_fidelity.clamp(0.0, 1.0),
        llm_response.scope_accuracy.clamp(0.0, 1.0),
        llm_response.interpolation_risk.clamp(0.0, 1.0),
    );
    let verdict = score.verdict();

    Ok(GroundingResult {
        bullet_text: bullet.text.clone(),
        source_entry_id: bullet.source_entry_id,
        verdict,
        score,
        rejection_reason: llm_response.rejection_reason,
    })
}

/// Attempts to rewrite a failed bullet using the LLM.
///
/// Called when `score_bullet` returns `Fail`. Provides the rejection reason so
/// the LLM can specifically address the violation (scope inflation, interpolation, etc.).
///
/// Returns a new `SimulatedBullet` with the same metadata but rewritten text.
/// On error, returns the original bullet unchanged.
pub async fn regenerate_single_bullet(
    bullet: &SimulatedBullet,
    source_entry: &ContextEntryRow,
    rejection_reason: &str,
    llm: &LlmClient,
) -> Result<SimulatedBullet, AppError> {
    let entry_data_json = serde_json::to_string(&source_entry.data)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to serialize entry data: {e}")))?;

    let prompt = build_rewrite_prompt(
        &bullet.text,
        rejection_reason,
        &entry_data_json,
        &source_entry.contribution_type,
    );

    let rewrite_system = "You are a resume bullet rewriter. You MUST return valid JSON only.\n\
        Rewrite the bullet so it is fully grounded in the source data and uses language \
        appropriate for the contribution type. Do NOT invent any details not in the source.\n\
        Return exactly: {\"text\": \"<rewritten bullet>\"}";

    let result: RewrittenBullet = match llm.call_json(&prompt, rewrite_system).await {
        Ok(r) => r,
        Err(e) => {
            warn!(
                bullet = %bullet.text.chars().take(60).collect::<String>(),
                error = %e,
                "Rewrite LLM call failed — keeping original bullet"
            );
            return Ok(bullet.clone());
        }
    };

    if result.text.trim().is_empty() {
        warn!("Rewrite returned empty text — keeping original bullet");
        return Ok(bullet.clone());
    }

    Ok(SimulatedBullet {
        text: result.text,
        source_entry_id: bullet.source_entry_id,
        section: bullet.section.clone(),
        verified_line_count: bullet.verified_line_count,
        jd_keywords_used: bullet.jd_keywords_used.clone(),
        was_adjusted: true,
        flagged_for_review: bullet.flagged_for_review,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

fn build_rewrite_prompt(
    bullet_text: &str,
    rejection_reason: &str,
    entry_data_json: &str,
    contribution_type: &str,
) -> String {
    format!(
        "Rewrite this resume bullet to fix the grounding issue.\n\n\
        ORIGINAL BULLET: {bullet_text}\n\n\
        REJECTION REASON: {rejection_reason}\n\n\
        CONTRIBUTION TYPE: {contribution_type}\n\n\
        SOURCE DATA (use ONLY these facts, do not invent):\n{entry_data_json}\n\n\
        Rules:\n\
        - For 'team_member': use 'Contributed to', 'Collaborated on', 'Implemented as part of team'\n\
        - For 'reviewer': use 'Reviewed', 'Evaluated', 'Assessed'\n\
        - Do NOT invent numbers or tools not in the source data\n\
        - Keep the bullet concise (1-2 lines)\n\n\
        Return JSON: {{\"text\": \"<rewritten bullet>\"}}"
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grounding_score_composite_correct_formula() {
        // composite = 0.40 * source_match + 0.30 * specificity + 0.20 * scope - 0.10 * interp
        let score = GroundingScore::compute(0.95, 0.90, 0.85, 0.05);
        let expected = 0.40 * 0.95 + 0.30 * 0.90 + 0.20 * 0.85 - 0.10 * 0.05;
        assert!(
            (score.composite - expected).abs() < 1e-5,
            "composite={} expected={}",
            score.composite,
            expected
        );
    }

    #[test]
    fn test_verdict_pass_at_0_80() {
        let score = GroundingScore::compute(0.95, 0.85, 0.90, 0.1);
        // 0.40*0.95 + 0.30*0.85 + 0.20*0.90 - 0.10*0.1 = 0.38 + 0.255 + 0.18 - 0.01 = 0.805
        assert!(
            score.composite >= 0.80,
            "composite should be ≥ 0.80, got {}",
            score.composite
        );
        assert_eq!(score.verdict(), GroundingVerdict::Pass);
    }

    #[test]
    fn test_verdict_fail_below_0_65() {
        // Low source_match + high interpolation → fail
        let score = GroundingScore::compute(0.3, 0.4, 0.5, 0.9);
        // 0.40*0.3 + 0.30*0.4 + 0.20*0.5 - 0.10*0.9 = 0.12 + 0.12 + 0.10 - 0.09 = 0.25
        assert!(
            score.composite < 0.65,
            "composite should be < 0.65, got {}",
            score.composite
        );
        assert_eq!(score.verdict(), GroundingVerdict::Fail);
    }
}
