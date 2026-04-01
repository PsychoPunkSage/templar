//! Layout Simulation Loop — enforces the Line Coverage Contract on draft bullets.
#![allow(dead_code)]
//!
//! # Architecture
//! - `run_simulation_loop` is the public async entry point. Max 3 passes.
//! - `run_single_pass_sync` is the CPU-bound inner pass, run via `tokio::task::spawn_blocking`.
//! - Between passes, async LLM calls fix violations (expand or compress).
//! - After 3 passes, remaining violators are flagged for human review.
//!
//! # spawn_blocking pattern
//! Width summation over all bullets is CPU-bound but fast. `spawn_blocking` keeps the
//! tokio scheduler unblocked. `run_single_pass_sync` accepts owned data (required for
//! 'static closure bounds) and returns only the violating indices + results.

use serde::{Deserialize, Serialize};
use tracing::warn;
use uuid::Uuid;

use crate::errors::AppError;
use crate::generation::generator::DraftBullet;
use crate::generation::jd_parser::ParsedJD;
use crate::layout::contract::{check_contract, LineCoverageResult, LineCoverageVerdict};
use crate::layout::font_metrics::{get_metrics, FontMetricTable, PageConfig};
use crate::layout::prompts::{
    COMPRESS_PROMPT_TEMPLATE, COMPRESS_SYSTEM, EXPAND_PROMPT_TEMPLATE, EXPAND_SYSTEM,
};
use crate::llm_client::LlmClient;

// ────────────────────────────────────────────────────────────────────────────
// Output types
// ────────────────────────────────────────────────────────────────────────────

/// A bullet after layout simulation. Replaces `DraftBullet` for all downstream consumers.
///
/// `verified_line_count` is set by the simulation — NOT the LLM's `line_estimate`.
/// `was_adjusted` indicates at least one LLM expand/compress call modified the text.
/// `flagged_for_review` indicates the bullet still violated the contract after max passes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulatedBullet {
    pub text: String,
    pub source_entry_id: Uuid,
    pub section: String,
    /// Line count as measured by the simulator (1 or 2 for passing bullets).
    pub verified_line_count: u8,
    pub jd_keywords_used: Vec<String>,
    /// True if the simulator called the LLM at least once to adjust this bullet.
    pub was_adjusted: bool,
    /// True if the bullet still violates the contract after all simulation passes.
    pub flagged_for_review: bool,
}

/// Summary of a complete simulation run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub bullets: Vec<SimulatedBullet>,
    pub total_passes: u8,
    pub violations_remaining: u32,
    pub flagged_count: u32,
    pub llm_calls_made: u32,
    /// Set by the page fill pass when minor overflow is best resolved by tightening spacing.
    pub tighten_spacing: bool,
}

/// Intermediate type for deserializing the LLM's adjust response.
#[derive(Debug, Deserialize)]
struct AdjustedBullet {
    text: String,
}

// ────────────────────────────────────────────────────────────────────────────
// Public entry point
// ────────────────────────────────────────────────────────────────────────────

const MAX_PASSES: u8 = 3;

/// Runs the layout simulation loop on a set of draft bullets.
///
/// Steps per pass:
/// 1. `spawn_blocking` → `run_single_pass_sync` (CPU-bound width check)
/// 2. For each violation: async LLM call to expand or compress
/// 3. Update bullet text in place
///
/// After MAX_PASSES, remaining violations are flagged for human review.
pub async fn run_simulation_loop(
    bullets: Vec<DraftBullet>,
    config: &PageConfig,
    parsed_jd: &ParsedJD,
    llm: &LlmClient,
) -> Result<SimulationResult, AppError> {
    let mut sim_bullets = init_simulated(bullets);
    let config_clone = config.clone();
    let mut total_passes = 0u8;
    let mut llm_calls_made = 0u32;

    for pass in 0..MAX_PASSES {
        total_passes += 1;

        // CPU-bound pass — spawn_blocking to avoid blocking the async executor.
        let bullets_snapshot = sim_bullets.clone();
        let cfg = config_clone.clone();
        let violations: Vec<(usize, LineCoverageResult)> = tokio::task::spawn_blocking(move || {
            let metrics = get_metrics(&cfg.font);
            run_single_pass_sync(&bullets_snapshot, metrics, &cfg)
        })
        .await
        .map_err(|e| {
            AppError::Internal(anyhow::anyhow!("spawn_blocking failed in simulation: {e}"))
        })?;

        if violations.is_empty() {
            break;
        }

        // Fix violations with LLM calls (async, not blocking).
        for (idx, coverage_result) in &violations {
            let bullet = &mut sim_bullets[*idx];
            let char_budget = estimate_char_budget(config);
            // On pass > 0, pass the current text as the failed previous attempt so the
            // LLM can see what it tried before and correct its approach.
            let prev = if pass == 0 {
                None
            } else {
                Some(bullet.text.as_str())
            };

            let adjusted_text = match &coverage_result.verdict {
                LineCoverageVerdict::TooShort { fill_ratio, .. } => {
                    llm_calls_made += 1;
                    expand_bullet(&bullet.text, *fill_ratio, char_budget, parsed_jd, llm, prev)
                        .await
                        .unwrap_or_else(|_| bullet.text.clone())
                }

                LineCoverageVerdict::TooLong { actual_lines } => {
                    llm_calls_made += 1;
                    compress_bullet(
                        &bullet.text,
                        *actual_lines,
                        char_budget,
                        parsed_jd,
                        llm,
                        prev,
                    )
                    .await
                    .unwrap_or_else(|_| bullet.text.clone())
                }

                LineCoverageVerdict::SecondLineTooShort { fill_ratio } => {
                    // Line 2 is too short — try expanding to fill it more.
                    llm_calls_made += 1;
                    expand_bullet(
                        &bullet.text,
                        *fill_ratio,
                        char_budget * 2, // 2-line budget
                        parsed_jd,
                        llm,
                        prev,
                    )
                    .await
                    .unwrap_or_else(|_| bullet.text.clone())
                }

                LineCoverageVerdict::Satisfies => bullet.text.clone(),
            };

            if adjusted_text != bullet.text {
                bullet.text = adjusted_text;
                bullet.was_adjusted = true;
            }
        }
    }

    // Final pass: determine verified_line_count and flag remaining violators.
    let bullets_final = sim_bullets.clone();
    let cfg_final = config_clone.clone();
    let final_violations: Vec<(usize, LineCoverageResult)> =
        tokio::task::spawn_blocking(move || {
            let metrics = get_metrics(&cfg_final.font);
            run_single_pass_sync(&bullets_final, metrics, &cfg_final)
        })
        .await
        .map_err(|e| {
            AppError::Internal(anyhow::anyhow!(
                "spawn_blocking failed in final simulation pass: {e}"
            ))
        })?;

    let violations_remaining = final_violations.len() as u32;
    let mut flagged_count = 0u32;

    for (idx, _) in &final_violations {
        sim_bullets[*idx].flagged_for_review = true;
        flagged_count += 1;
    }

    if flagged_count > 0 {
        warn!(
            flagged = flagged_count,
            passes = total_passes,
            "Layout simulation: bullets still violating contract after max passes"
        );
    }

    // Update verified_line_count for all bullets via a final blocking measurement.
    let bullets_snapshot = sim_bullets.clone();
    let cfg_measure = config_clone.clone();
    let line_counts: Vec<u8> = tokio::task::spawn_blocking(move || {
        let metrics = get_metrics(&cfg_measure.font);
        bullets_snapshot
            .iter()
            .map(|b| {
                let (count, _) =
                    crate::layout::contract::simulate_lines(&b.text, metrics, &cfg_measure);
                count.max(1) // treat empty string as 1 line
            })
            .collect()
    })
    .await
    .map_err(|e| {
        AppError::Internal(anyhow::anyhow!(
            "spawn_blocking failed updating line counts: {e}"
        ))
    })?;

    for (bullet, &count) in sim_bullets.iter_mut().zip(line_counts.iter()) {
        bullet.verified_line_count = count;
    }

    // ── Enforce 2-line promotion eligibility rules ──────────────────────────
    // Block A: If a bullet occupies 2 lines but doesn't meet promotion criteria,
    // attempt to compress it to 1 line.
    for bullet in &mut sim_bullets {
        //for i in 0..sim_bullets.len() {
        if bullet.verified_line_count != 2 {
            continue;
        }
        let draft = crate::generation::generator::DraftBullet {
            text: bullet.text.clone(),
            source_entry_id: bullet.source_entry_id,
            section: bullet.section.clone(),
            line_estimate: 2,
            jd_keywords_used: bullet.jd_keywords_used.clone(),
        };
        let promo = crate::layout::contract::score_promotion(&draft, parsed_jd);
        if !promo.eligible_for_two_lines {
            let char_budget = estimate_char_budget(config);
            match compress_bullet(&bullet.text, 2, char_budget, parsed_jd, llm, None).await {
                Ok(compressed) => {
                    bullet.text = compressed;
                    bullet.was_adjusted = true;
                    llm_calls_made += 1;
                    let metrics = crate::layout::font_metrics::get_metrics(&config.font);
                    let (new_count, _) =
                        crate::layout::contract::simulate_lines(&bullet.text, metrics, config);
                    bullet.verified_line_count = new_count.max(1);
                    if bullet.verified_line_count == 2 {
                        bullet.flagged_for_review = true;
                    }
                }
                Err(_) => {
                    bullet.flagged_for_review = true;
                }
            }
        }
    }

    // ── Enforce max 3 two-line bullets per page ──────────────────────────────
    // Block B: If more than 3 bullets are 2-line and not flagged, compress the
    // lowest-JD-relevance one until the cap is respected.
    const MAX_TWO_LINE: usize = 3;
    loop {
        let two_line_indices: Vec<usize> = sim_bullets
            .iter()
            .enumerate()
            .filter(|(_, b)| b.verified_line_count == 2 && !b.flagged_for_review)
            .map(|(i, _)| i)
            .collect();
        if two_line_indices.len() <= MAX_TWO_LINE {
            break;
        }
        let jd_kw_set: std::collections::HashSet<String> = parsed_jd
            .keyword_inventory
            .iter()
            .map(|k| k.keyword.to_lowercase())
            .collect();
        let target_idx = *two_line_indices
            .iter()
            .min_by(|&&a, &&b| {
                let sa = sim_bullets[a]
                    .jd_keywords_used
                    .iter()
                    .filter(|kw| jd_kw_set.contains(&kw.to_lowercase()))
                    .count();
                let sb = sim_bullets[b]
                    .jd_keywords_used
                    .iter()
                    .filter(|kw| jd_kw_set.contains(&kw.to_lowercase()))
                    .count();
                sa.cmp(&sb)
            })
            .unwrap();
        let char_budget = estimate_char_budget(config);
        match compress_bullet(
            &sim_bullets[target_idx].text,
            2,
            char_budget,
            parsed_jd,
            llm,
            None,
        )
        .await
        {
            Ok(compressed) => {
                sim_bullets[target_idx].text = compressed;
                sim_bullets[target_idx].was_adjusted = true;
                llm_calls_made += 1;
                let metrics = crate::layout::font_metrics::get_metrics(&config.font);
                let (new_count, _) = crate::layout::contract::simulate_lines(
                    &sim_bullets[target_idx].text,
                    metrics,
                    config,
                );
                sim_bullets[target_idx].verified_line_count = new_count.max(1);
                if sim_bullets[target_idx].verified_line_count == 2 {
                    // Compression failed — flag and break to prevent infinite loop
                    sim_bullets[target_idx].flagged_for_review = true;
                    break;
                }
            }
            Err(_) => {
                sim_bullets[target_idx].flagged_for_review = true;
                break;
            }
        }
    }

    Ok(SimulationResult {
        bullets: sim_bullets,
        total_passes,
        violations_remaining,
        flagged_count,
        llm_calls_made,
        tighten_spacing: false,
    })
}

// ────────────────────────────────────────────────────────────────────────────
// Synchronous inner pass (runs inside spawn_blocking)
// ────────────────────────────────────────────────────────────────────────────

/// Runs one simulation pass over all bullets.
///
/// Returns `(bullet_index, LineCoverageResult)` for each violating bullet only.
/// Takes owned/borrowed data compatible with `spawn_blocking`'s `'static` requirement
/// (callers pass cloned `Vec<SimulatedBullet>` and `PageConfig`).
pub(crate) fn run_single_pass_sync(
    bullets: &[SimulatedBullet],
    metrics: &FontMetricTable,
    config: &PageConfig,
) -> Vec<(usize, LineCoverageResult)> {
    bullets
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            let result = check_contract(i, &b.text, metrics, config);
            if matches!(result.verdict, LineCoverageVerdict::Satisfies) {
                None
            } else {
                Some((i, result))
            }
        })
        .collect()
}

// ────────────────────────────────────────────────────────────────────────────
// LLM adjust calls
// ────────────────────────────────────────────────────────────────────────────

/// Calls the LLM to expand a bullet that doesn't fill enough horizontal space.
pub(crate) async fn expand_bullet(
    text: &str,
    fill_ratio: f32,
    char_budget: usize,
    parsed_jd: &ParsedJD,
    llm: &LlmClient,
    previous_attempt: Option<&str>,
) -> Result<String, AppError> {
    let prompt = build_expand_prompt(text, fill_ratio, char_budget, parsed_jd, previous_attempt);
    let result: AdjustedBullet = llm
        .call_json(&prompt, EXPAND_SYSTEM)
        .await
        .map_err(|e| AppError::Llm(format!("Expand LLM call failed: {e}")))?;
    Ok(result.text)
}

/// Calls the LLM to compress a bullet that wraps to 3+ lines.
pub(crate) async fn compress_bullet(
    text: &str,
    actual_lines: u8,
    char_budget: usize,
    parsed_jd: &ParsedJD,
    llm: &LlmClient,
    previous_attempt: Option<&str>,
) -> Result<String, AppError> {
    let prompt =
        build_compress_prompt(text, actual_lines, char_budget, parsed_jd, previous_attempt);
    let result: AdjustedBullet = llm
        .call_json(&prompt, COMPRESS_SYSTEM)
        .await
        .map_err(|e| AppError::Llm(format!("Compress LLM call failed: {e}")))?;
    Ok(result.text)
}

// ────────────────────────────────────────────────────────────────────────────
// Prompt builders
// ────────────────────────────────────────────────────────────────────────────

pub(crate) fn build_expand_prompt(
    text: &str,
    fill_ratio: f32,
    char_budget: usize,
    parsed_jd: &ParsedJD,
    previous_attempt: Option<&str>,
) -> String {
    let jd_keywords = top_jd_keywords(parsed_jd, 5);
    let min_char_budget = ((char_budget as f32) * 0.85).max(1.0) as usize;
    EXPAND_PROMPT_TEMPLATE
        .replace("{bullet_text}", text)
        .replace("{fill_percent}", &format!("{:.0}", fill_ratio * 100.0))
        .replace("{required_percent}", "80")
        .replace("{char_budget}", &char_budget.to_string())
        .replace("{min_char_budget}", &min_char_budget.to_string())
        .replace("{jd_keywords}", &jd_keywords)
        .replace(
            "{previous_attempt}",
            &match previous_attempt {
                Some(prev) => format!("PREVIOUS ATTEMPT (still failed): {}\n", prev),
                None => String::new(),
            },
        )
}

pub(crate) fn build_compress_prompt(
    text: &str,
    actual_lines: u8,
    char_budget: usize,
    parsed_jd: &ParsedJD,
    previous_attempt: Option<&str>,
) -> String {
    let jd_keywords = top_jd_keywords(parsed_jd, 5);
    COMPRESS_PROMPT_TEMPLATE
        .replace("{bullet_text}", text)
        .replace("{actual_lines}", &actual_lines.to_string())
        .replace("{char_budget}", &char_budget.to_string())
        .replace("{jd_keywords}", &jd_keywords)
        .replace(
            "{previous_attempt}",
            &match previous_attempt {
                Some(prev) => format!("PREVIOUS ATTEMPT (still failed): {}\n", prev),
                None => String::new(),
            },
        )
}

// ────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ────────────────────────────────────────────────────────────────────────────

/// Converts a `Vec<DraftBullet>` into the initial `Vec<SimulatedBullet>` for simulation.
pub(crate) fn init_simulated(bullets: Vec<DraftBullet>) -> Vec<SimulatedBullet> {
    bullets
        .into_iter()
        .map(|b| SimulatedBullet {
            text: b.text,
            source_entry_id: b.source_entry_id,
            section: b.section,
            verified_line_count: b.line_estimate, // will be overwritten by simulation
            jd_keywords_used: b.jd_keywords_used,
            was_adjusted: false,
            flagged_for_review: false,
        })
        .collect()
}

/// Estimates the maximum character count for a 1-line bullet at the current config.
pub(crate) fn estimate_char_budget(config: &PageConfig) -> usize {
    let metrics = get_metrics(&config.font);
    // text_width_em / average_char_width gives approximate chars per line
    (config.text_width_em / metrics.average_char_width).round() as usize
}

/// Returns the top N JD keywords by weighted_score, comma-separated.
fn top_jd_keywords(parsed_jd: &ParsedJD, n: usize) -> String {
    let mut keywords: Vec<&str> = parsed_jd
        .keyword_inventory
        .iter()
        .map(|k| k.keyword.as_str())
        .collect();
    // Take up to n keywords (already sorted by weighted_score in most JD parsers)
    keywords.truncate(n);
    if keywords.is_empty() {
        "none specified".to_string()
    } else {
        keywords.join(", ")
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::jd_parser::{JDTone, KeywordEntry, ParsedJD, Requirement, RoleSignals};
    use crate::layout::contract::LineCoverageVerdict;
    use crate::layout::font_metrics::{default_page_config, get_metrics, FontFamily};
    use uuid::Uuid;

    fn make_page_config() -> PageConfig {
        default_page_config(FontFamily::Inter)
    }

    fn make_parsed_jd() -> ParsedJD {
        ParsedJD {
            hard_requirements: vec![Requirement {
                text: "Rust".to_string(),
                is_required: true,
            }],
            soft_signals: vec![],
            role_signals: RoleSignals {
                is_startup: true,
                is_ic_focused: true,
                is_research: false,
                seniority: "senior".to_string(),
            },
            keyword_inventory: vec![
                KeywordEntry {
                    keyword: "Rust".to_string(),
                    frequency: 5,
                    position_weight: 0.8,
                    weighted_score: 4.0,
                },
                KeywordEntry {
                    keyword: "distributed".to_string(),
                    frequency: 3,
                    position_weight: 0.6,
                    weighted_score: 1.8,
                },
            ],
            detected_tone: JDTone::AggressiveStartup,
        }
    }

    fn make_draft_bullet(text: &str) -> DraftBullet {
        DraftBullet {
            text: text.to_string(),
            source_entry_id: Uuid::new_v4(),
            section: "experience".to_string(),
            line_estimate: 1,
            jd_keywords_used: vec!["Rust".to_string()],
        }
    }

    // ── init_simulated ──────────────────────────────────────────────────────

    #[test]
    fn test_init_simulated_preserves_fields() {
        let id = Uuid::new_v4();
        let draft = DraftBullet {
            text: "Built a system".to_string(),
            source_entry_id: id,
            section: "experience".to_string(),
            line_estimate: 1,
            jd_keywords_used: vec!["Rust".to_string()],
        };

        let sim = init_simulated(vec![draft]);
        assert_eq!(sim.len(), 1);
        assert_eq!(sim[0].text, "Built a system");
        assert_eq!(sim[0].source_entry_id, id);
        assert_eq!(sim[0].section, "experience");
        assert!(!sim[0].was_adjusted);
        assert!(!sim[0].flagged_for_review);
    }

    #[test]
    fn test_init_simulated_empty_input() {
        let sim = init_simulated(vec![]);
        assert!(sim.is_empty());
    }

    // ── run_single_pass_sync ────────────────────────────────────────────────

    #[test]
    fn test_single_pass_empty_bullets_no_violations() {
        let config = make_page_config();
        let metrics = get_metrics(&config.font);
        let violations = run_single_pass_sync(&[], metrics, &config);
        assert!(violations.is_empty());
    }

    #[test]
    fn test_single_pass_short_bullet_returns_violation() {
        let config = make_page_config();
        let metrics = get_metrics(&config.font);

        let bullet = SimulatedBullet {
            text: "Built it.".to_string(),
            source_entry_id: Uuid::new_v4(),
            section: "experience".to_string(),
            verified_line_count: 1,
            jd_keywords_used: vec![],
            was_adjusted: false,
            flagged_for_review: false,
        };

        let violations = run_single_pass_sync(&[bullet], metrics, &config);
        assert_eq!(violations.len(), 1, "short bullet should be a violation");
        assert!(matches!(
            violations[0].1.verdict,
            LineCoverageVerdict::TooShort { .. }
        ));
    }

    #[test]
    fn test_single_pass_long_bullet_returns_violation() {
        let config = make_page_config();
        let metrics = get_metrics(&config.font);

        let long_text = "word ".repeat(50);
        let bullet = SimulatedBullet {
            text: long_text,
            source_entry_id: Uuid::new_v4(),
            section: "experience".to_string(),
            verified_line_count: 1,
            jd_keywords_used: vec![],
            was_adjusted: false,
            flagged_for_review: false,
        };

        let violations = run_single_pass_sync(&[bullet], metrics, &config);
        assert_eq!(violations.len(), 1, "long bullet should be a violation");
        assert!(matches!(
            violations[0].1.verdict,
            LineCoverageVerdict::TooLong { .. }
        ));
    }

    // ── prompt builders ─────────────────────────────────────────────────────

    #[test]
    fn test_build_expand_prompt_contains_bullet_text() {
        let jd = make_parsed_jd();
        let prompt = build_expand_prompt("Built a system", 0.45, 82, &jd, None);
        assert!(
            prompt.contains("Built a system"),
            "prompt should contain original bullet"
        );
        assert!(
            prompt.contains("45%"),
            "prompt should contain fill percentage"
        );
        assert!(prompt.contains("82"), "prompt should contain char budget");
    }

    #[test]
    fn test_build_compress_prompt_contains_line_count() {
        let jd = make_parsed_jd();
        let prompt =
            build_compress_prompt("A very long bullet that goes on and on", 4, 164, &jd, None);
        assert!(
            prompt.contains("4"),
            "prompt should contain actual line count"
        );
        assert!(prompt.contains("164"), "prompt should contain char budget");
    }

    #[test]
    fn test_build_expand_prompt_includes_jd_keywords() {
        let jd = make_parsed_jd();
        let prompt = build_expand_prompt("Did work", 0.30, 82, &jd, None);
        assert!(
            prompt.contains("Rust") || prompt.contains("distributed"),
            "prompt should include JD keywords"
        );
    }

    #[test]
    fn test_build_compress_prompt_hard_ceiling() {
        let jd = make_parsed_jd();
        let prompt = build_compress_prompt("some bullet", 3, 100, &jd, None);
        assert!(
            prompt.contains("MUST NOT exceed"),
            "compress prompt must use hard ceiling language"
        );
        assert!(
            !prompt.contains("approximately"),
            "compress prompt must not use approximate language"
        );
    }

    #[test]
    fn test_build_expand_prompt_with_previous_attempt() {
        let jd = make_parsed_jd();
        let prompt = build_expand_prompt("short bullet", 0.5, 120, &jd, Some("old attempt text"));
        assert!(
            prompt.contains("PREVIOUS ATTEMPT (still failed): old attempt text"),
            "prompt must include previous attempt"
        );
    }

    #[test]
    fn test_build_compress_prompt_with_previous_attempt() {
        let jd = make_parsed_jd();
        let prompt = build_compress_prompt("long bullet", 3, 100, &jd, Some("previous long text"));
        assert!(
            prompt.contains("PREVIOUS ATTEMPT (still failed): previous long text"),
            "prompt must include previous attempt"
        );
    }

    #[test]
    fn test_build_expand_prompt_min_char_budget() {
        let jd = make_parsed_jd();
        let prompt = build_expand_prompt("bullet", 0.5, 120, &jd, None);
        // min = floor(120 * 0.85) = 102
        assert!(
            prompt.contains("102"),
            "prompt must contain min char budget 102"
        );
        assert!(
            prompt.contains("120"),
            "prompt must contain max char budget 120"
        );
    }

    // ── flagged_for_review after max passes ─────────────────────────────────

    #[test]
    fn test_init_simulated_flagged_starts_false() {
        let bullet = make_draft_bullet("Test bullet");
        let sim = init_simulated(vec![bullet]);
        assert!(!sim[0].flagged_for_review);
    }
}
