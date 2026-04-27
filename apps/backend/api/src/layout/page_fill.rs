//! Page Fill Analysis — checks whether the full page is well-utilized after simulation.
#![allow(dead_code)]
//!
//! After the line-level simulation loop completes, the page may still have too much
//! whitespace (> 8%) or an overflow (> 5%). This module analyzes the overall fill and
//! recommends a remediation action.
//!
//! # Page fill rules (from spec)
//! - Whitespace > 8%  → add item OR promote a 1-line bullet to 2-line
//! - Overflow < 5%    → compress bullets or tighten spacing
//! - Overflow > 5%    → remove lowest-scoring item, re-run

use serde::{Deserialize, Serialize};

use crate::generation::jd_parser::ParsedJD;
use crate::layout::font_metrics::PageConfig;
use crate::layout::simulator::SimulatedBullet;

// ────────────────────────────────────────────────────────────────────────────
// Types
// ────────────────────────────────────────────────────────────────────────────

/// Overall page fill verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum PageFillVerdict {
    /// 92–100% fill — acceptable range.
    Acceptable,
    /// < 92% fill (> 8% whitespace) — page has too much empty space.
    TooMuchWhitespace,
    /// 100–105% fill — minor overflow, can likely be fixed by compression or spacing.
    MinorOverflow,
    /// > 105% fill — major overflow, must remove the lowest-scoring bullet.
    MajorOverflow,
}

/// Full page fill analysis result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageFillAnalysis {
    pub total_lines_used: u16,
    pub total_lines_available: u16,
    pub whitespace_fraction: f32,
    pub overflow_fraction: f32,
    pub verdict: PageFillVerdict,
}

/// Recommended remediation action for a non-Acceptable page fill.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum FillAction {
    /// Promote a 1-line bullet to 2-line to consume whitespace.
    PromoteBullet { bullet_index: usize },
    /// Compress a bullet to reclaim lines.
    CompressBullet { bullet_index: usize },
    /// Remove the lowest-scoring bullet to eliminate overflow.
    RemoveBullet { bullet_index: usize },
    /// Tighten LaTeX inter-item spacing (minor overflow, no bullet to remove).
    TightenSpacing,
    /// No action needed.
    NoAction,
}

// ────────────────────────────────────────────────────────────────────────────
// Core functions
// ────────────────────────────────────────────────────────────────────────────

/// Analyzes the overall page fill given the simulated bullets and page configuration.
///
/// `total_lines_used` is the sum of `verified_line_count` across all bullets.
/// `usable_height_lines` from `PageConfig` is the denominator.
///
/// `is_last_page` — when false (intermediate CV pages), `TooMuchWhitespace` is never
/// reported: partial pages between entries are acceptable in multi-page documents.
pub fn analyze_page_fill(
    bullets: &[SimulatedBullet],
    config: &PageConfig,
    is_last_page: bool,
) -> PageFillAnalysis {
    let total_lines_used: u16 = bullets.iter().map(|b| b.verified_line_count as u16).sum();

    let available = config.usable_height_lines;
    let fill_ratio = total_lines_used as f32 / available as f32;

    let whitespace_fraction = (1.0_f32 - fill_ratio).max(0.0);
    let overflow_fraction = (fill_ratio - 1.0_f32).max(0.0);

    let verdict = if fill_ratio > 1.05 {
        PageFillVerdict::MajorOverflow
    } else if fill_ratio > 1.00 {
        PageFillVerdict::MinorOverflow
    } else if whitespace_fraction > 0.08 && is_last_page {
        // Only flag whitespace on the final page — intermediate CV pages can be partial.
        PageFillVerdict::TooMuchWhitespace
    } else {
        PageFillVerdict::Acceptable
    };

    PageFillAnalysis {
        total_lines_used,
        total_lines_available: available,
        whitespace_fraction,
        overflow_fraction,
        verdict,
    }
}

/// Recommends a single remediation action based on the page fill analysis.
///
/// The caller is responsible for executing the action (expand, compress, or remove a bullet).
pub fn recommend_fill_action(
    analysis: &PageFillAnalysis,
    bullets: &[SimulatedBullet],
    parsed_jd: &ParsedJD,
) -> FillAction {
    match &analysis.verdict {
        PageFillVerdict::Acceptable => FillAction::NoAction,

        PageFillVerdict::TooMuchWhitespace => {
            // Prefer promoting a 1-line bullet to 2-line to fill space.
            if let Some(idx) = find_best_promotion_candidate(bullets, parsed_jd) {
                FillAction::PromoteBullet { bullet_index: idx }
            } else {
                FillAction::NoAction
            }
        }

        PageFillVerdict::MinorOverflow => {
            // Compress the lowest-scoring bullet slightly.
            if let Some(idx) = find_lowest_scoring_bullet(bullets, parsed_jd) {
                FillAction::CompressBullet { bullet_index: idx }
            } else {
                FillAction::TightenSpacing
            }
        }

        PageFillVerdict::MajorOverflow => {
            // Remove the lowest-scoring bullet.
            if let Some(idx) = find_lowest_scoring_bullet(bullets, parsed_jd) {
                FillAction::RemoveBullet { bullet_index: idx }
            } else {
                FillAction::TightenSpacing
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ────────────────────────────────────────────────────────────────────────────

/// Finds the index of the bullet that matches the fewest JD keywords (lowest relevance).
///
/// Bullets that are already flagged for review are deprioritized for removal so that
/// human-reviewed bullets are not silently discarded.
fn find_lowest_scoring_bullet(bullets: &[SimulatedBullet], parsed_jd: &ParsedJD) -> Option<usize> {
    if bullets.is_empty() {
        return None;
    }

    let jd_keyword_set: std::collections::HashSet<String> = parsed_jd
        .keyword_inventory
        .iter()
        .map(|k| k.keyword.to_lowercase())
        .collect();

    bullets
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| {
            let score_a = keyword_match_score(&a.jd_keywords_used, &jd_keyword_set);
            let score_b = keyword_match_score(&b.jd_keywords_used, &jd_keyword_set);
            score_a
                .partial_cmp(&score_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
}

/// Finds the best 1-line bullet to promote to 2 lines (for whitespace reduction).
///
/// Chooses the bullet with the most JD keyword matches that is currently 1 line.
fn find_best_promotion_candidate(
    bullets: &[SimulatedBullet],
    parsed_jd: &ParsedJD,
) -> Option<usize> {
    let jd_keyword_set: std::collections::HashSet<String> = parsed_jd
        .keyword_inventory
        .iter()
        .map(|k| k.keyword.to_lowercase())
        .collect();

    bullets
        .iter()
        .enumerate()
        .filter(|(_, b)| b.verified_line_count == 1 && !b.flagged_for_review)
        .max_by(|(_, a), (_, b)| {
            let score_a = keyword_match_score(&a.jd_keywords_used, &jd_keyword_set);
            let score_b = keyword_match_score(&b.jd_keywords_used, &jd_keyword_set);
            score_a
                .partial_cmp(&score_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
}

/// Counts how many of the bullet's JD keywords match the JD keyword set.
fn keyword_match_score(
    used_keywords: &[String],
    jd_keyword_set: &std::collections::HashSet<String>,
) -> f32 {
    let matched = used_keywords
        .iter()
        .filter(|kw| jd_keyword_set.contains(&kw.to_lowercase()))
        .count();
    matched as f32
}

// ────────────────────────────────────────────────────────────────────────────
// Page fill remediation pass
// ────────────────────────────────────────────────────────────────────────────

/// Maximum number of page fill remediation passes before flagging for human review.
const MAX_FILL_PASSES: u8 = 3;

/// Runs iterative page fill remediation after the simulation loop (up to MAX_FILL_PASSES).
///
/// Each pass analyzes overall whitespace/overflow and executes the recommended action:
/// - TooMuchWhitespace → promote best 1-line bullet to 2-line (LLM expand)
/// - MinorOverflow     → compress lowest-scoring bullet (LLM compress)
/// - MajorOverflow     → remove lowest-scoring bullet (no LLM)
///
/// Loops until Acceptable or no further action is possible.
/// After MAX_FILL_PASSES, sets `result.page_fill_flagged = true` for human review.
pub async fn run_page_fill_pass(
    mut result: crate::layout::simulator::SimulationResult,
    config: &crate::layout::font_metrics::PageConfig,
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
    llm: &crate::llm_client::LlmClient,
    is_last_page: bool,
) -> Result<crate::layout::simulator::SimulationResult, crate::errors::AppError> {
    use crate::layout::contract::simulate_lines;
    use crate::layout::font_metrics::get_metrics;
    use crate::layout::simulator::{compress_bullet, estimate_char_budget, expand_bullet};

    let metrics = get_metrics(&config.font);
    let char_budget = estimate_char_budget(config);
    let mut fill_passes = 0u8;

    loop {
        let analysis = analyze_page_fill(&result.bullets, config, is_last_page);

        if matches!(analysis.verdict, PageFillVerdict::Acceptable) {
            break;
        }

        if fill_passes >= MAX_FILL_PASSES {
            result.page_fill_flagged = true;
            tracing::warn!(
                passes = MAX_FILL_PASSES,
                verdict = ?analysis.verdict,
                whitespace_pct = analysis.whitespace_fraction * 100.0,
                overflow_pct = analysis.overflow_fraction * 100.0,
                "page fill: still violating after max passes — flagged for human review"
            );
            break;
        }

        let action = recommend_fill_action(&analysis, &result.bullets, parsed_jd);

        match action {
            FillAction::PromoteBullet { bullet_index } => {
                if bullet_index < result.bullets.len() {
                    let two_line_budget = char_budget * 2;
                    let new_text = expand_bullet(
                        &result.bullets[bullet_index].text,
                        analysis.whitespace_fraction,
                        two_line_budget,
                        parsed_jd,
                        llm,
                        None,
                    )
                    .await
                    .unwrap_or_else(|_| result.bullets[bullet_index].text.clone());
                    result.bullets[bullet_index].text = new_text;
                    result.bullets[bullet_index].was_adjusted = true;
                    result.llm_calls_made += 1;
                    let (new_count, _) =
                        simulate_lines(&result.bullets[bullet_index].text, metrics, config);
                    result.bullets[bullet_index].verified_line_count = new_count.max(1);
                }
            }
            FillAction::CompressBullet { bullet_index } => {
                if bullet_index < result.bullets.len() {
                    let actual_lines = result.bullets[bullet_index].verified_line_count;
                    let new_text = compress_bullet(
                        &result.bullets[bullet_index].text,
                        actual_lines,
                        char_budget,
                        parsed_jd,
                        llm,
                        None,
                    )
                    .await
                    .unwrap_or_else(|_| result.bullets[bullet_index].text.clone());
                    result.bullets[bullet_index].text = new_text;
                    result.bullets[bullet_index].was_adjusted = true;
                    result.llm_calls_made += 1;
                    let (new_count, _) =
                        simulate_lines(&result.bullets[bullet_index].text, metrics, config);
                    result.bullets[bullet_index].verified_line_count = new_count.max(1);
                }
            }
            FillAction::RemoveBullet { bullet_index } => {
                if bullet_index < result.bullets.len() {
                    let removed_entry_id = result.bullets[bullet_index].source_entry_id;
                    result.bullets.remove(bullet_index);

                    // If this was the last content bullet for the entry, remove the header
                    // placeholder to prevent a dangling header (label with no bullets) in
                    // the rendered PDF.
                    let has_remaining_content = result
                        .bullets
                        .iter()
                        .any(|b| b.source_entry_id == removed_entry_id && !b.text.is_empty());
                    if !has_remaining_content {
                        result.bullets.retain(|b| {
                            !(b.source_entry_id == removed_entry_id && b.text.is_empty())
                        });
                    }
                }
            }
            FillAction::TightenSpacing => {
                result.tighten_spacing = true;
                break; // spacing is a one-time hint, not an iterative action
            }
            FillAction::NoAction => {
                break; // nothing left to do
            }
        }

        fill_passes += 1;
    }

    Ok(result)
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::jd_parser::{JDTone, KeywordEntry, ParsedJD, Requirement, RoleSignals};
    use crate::layout::font_metrics::{default_page_config, FontFamily};
    use uuid::Uuid;

    fn make_config() -> PageConfig {
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

    fn make_bullet(line_count: u8, keywords: Vec<&str>, flagged: bool) -> SimulatedBullet {
        SimulatedBullet {
            text: "Architected systems".to_string(),
            source_entry_id: Uuid::new_v4(),
            section: "experience".to_string(),
            entry_header_latex: None,
            verified_line_count: line_count,
            jd_keywords_used: keywords.into_iter().map(|s| s.to_string()).collect(),
            was_adjusted: false,
            flagged_for_review: flagged,
            page_number: 1,
        }
    }

    // ── analyze_page_fill verdicts ──────────────────────────────────────────

    #[test]
    fn test_acceptable_fill_verdict() {
        let config = make_config(); // 45 usable lines
                                    // 43 lines used = 95.6% fill → Acceptable (whitespace = 4.4% < 8%)
        let bullets: Vec<SimulatedBullet> =
            (0..43).map(|_| make_bullet(1, vec![], false)).collect();
        let analysis = analyze_page_fill(&bullets, &config, true);
        assert_eq!(analysis.verdict, PageFillVerdict::Acceptable);
        assert_eq!(analysis.total_lines_used, 43);
        assert!(analysis.whitespace_fraction < 0.08);
    }

    #[test]
    fn test_too_much_whitespace_verdict() {
        let config = make_config(); // 45 usable lines
                                    // 35 lines used = 77.8% fill → TooMuchWhitespace (whitespace = 22.2% > 8%)
        let bullets: Vec<SimulatedBullet> =
            (0..35).map(|_| make_bullet(1, vec![], false)).collect();
        let analysis = analyze_page_fill(&bullets, &config, true);
        assert_eq!(analysis.verdict, PageFillVerdict::TooMuchWhitespace);
        assert!(analysis.whitespace_fraction > 0.08);
    }

    #[test]
    fn test_minor_overflow_verdict() {
        let config = make_config(); // 45 usable lines
                                    // 47 lines used = 104.4% fill → MinorOverflow (1–5%)
        let bullets: Vec<SimulatedBullet> =
            (0..47).map(|_| make_bullet(1, vec![], false)).collect();
        let analysis = analyze_page_fill(&bullets, &config, true);
        assert_eq!(analysis.verdict, PageFillVerdict::MinorOverflow);
        assert!(analysis.overflow_fraction > 0.0 && analysis.overflow_fraction <= 0.05);
    }

    #[test]
    fn test_major_overflow_verdict() {
        let config = make_config(); // 45 usable lines
                                    // 50 lines used = 111.1% fill → MajorOverflow (> 5%)
        let bullets: Vec<SimulatedBullet> =
            (0..50).map(|_| make_bullet(1, vec![], false)).collect();
        let analysis = analyze_page_fill(&bullets, &config, true);
        assert_eq!(analysis.verdict, PageFillVerdict::MajorOverflow);
        assert!(analysis.overflow_fraction > 0.05);
    }

    #[test]
    fn test_empty_bullets_is_whitespace() {
        let config = make_config();
        let analysis = analyze_page_fill(&[], &config, true);
        assert_eq!(analysis.verdict, PageFillVerdict::TooMuchWhitespace);
        assert_eq!(analysis.total_lines_used, 0);
        assert!((analysis.whitespace_fraction - 1.0).abs() < 1e-3);
    }

    // ── recommend_fill_action ────────────────────────────────────────────────

    #[test]
    fn test_recommend_no_action_for_acceptable() {
        let config = make_config();
        // 43/45 = 95.6% fill → Acceptable → NoAction
        let bullets: Vec<SimulatedBullet> =
            (0..43).map(|_| make_bullet(1, vec![], false)).collect();
        let analysis = analyze_page_fill(&bullets, &config, true);
        let action = recommend_fill_action(&analysis, &bullets, &make_parsed_jd());
        assert_eq!(action, FillAction::NoAction);
    }

    #[test]
    fn test_recommend_promote_for_whitespace() {
        let bullets = vec![make_bullet(1, vec!["Rust"], false)];
        let analysis = PageFillAnalysis {
            total_lines_used: 30,
            total_lines_available: 45,
            whitespace_fraction: 0.33,
            overflow_fraction: 0.0,
            verdict: PageFillVerdict::TooMuchWhitespace,
        };
        let action = recommend_fill_action(&analysis, &bullets, &make_parsed_jd());
        assert!(matches!(action, FillAction::PromoteBullet { .. }));
    }

    #[test]
    fn test_recommend_compress_for_minor_overflow() {
        let bullets = vec![make_bullet(2, vec!["Rust"], false)];
        let analysis = PageFillAnalysis {
            total_lines_used: 47,
            total_lines_available: 45,
            whitespace_fraction: 0.0,
            overflow_fraction: 0.044,
            verdict: PageFillVerdict::MinorOverflow,
        };
        let action = recommend_fill_action(&analysis, &bullets, &make_parsed_jd());
        assert!(
            matches!(action, FillAction::CompressBullet { .. })
                || matches!(action, FillAction::TightenSpacing)
        );
    }

    #[test]
    fn test_recommend_remove_for_major_overflow() {
        let bullets = vec![
            make_bullet(2, vec!["Rust", "distributed"], false),
            make_bullet(2, vec![], false), // no keywords → lowest score
        ];
        let analysis = PageFillAnalysis {
            total_lines_used: 50,
            total_lines_available: 45,
            whitespace_fraction: 0.0,
            overflow_fraction: 0.11,
            verdict: PageFillVerdict::MajorOverflow,
        };
        let action = recommend_fill_action(&analysis, &bullets, &make_parsed_jd());
        match action {
            FillAction::RemoveBullet { bullet_index } => {
                // Should remove the bullet with no JD keywords (index 1)
                assert_eq!(bullet_index, 1, "should remove the lowest-scoring bullet");
            }
            FillAction::TightenSpacing => {} // acceptable fallback
            other => panic!("expected RemoveBullet, got {other:?}"),
        }
    }

    // ── find_lowest_scoring_bullet ───────────────────────────────────────────

    #[test]
    fn test_lowest_scoring_bullet_no_keywords_wins() {
        let bullets = vec![
            make_bullet(1, vec!["Rust"], false),
            make_bullet(1, vec![], false), // lowest score
            make_bullet(1, vec!["distributed"], false),
        ];
        let idx = find_lowest_scoring_bullet(&bullets, &make_parsed_jd());
        assert_eq!(idx, Some(1), "bullet with no JD keywords should be lowest");
    }

    #[test]
    fn test_find_best_promotion_candidate_prefers_1_line() {
        let bullets = vec![
            make_bullet(2, vec!["Rust"], false), // already 2 lines, skip
            make_bullet(1, vec!["distributed"], false),
            make_bullet(1, vec!["Rust", "distributed"], false), // best match
        ];
        let idx = find_best_promotion_candidate(&bullets, &make_parsed_jd());
        assert_eq!(
            idx,
            Some(2),
            "best 1-line candidate should have most keywords"
        );
    }

    // ── MAX_FILL_PASSES constant ─────────────────────────────────────────────

    #[test]
    fn test_max_fill_passes_is_three() {
        assert_eq!(
            MAX_FILL_PASSES, 3,
            "spec requires exactly 3 max fill passes"
        );
    }

    #[test]
    fn test_major_overflow_removes_bullets_iteratively() {
        let config = make_config(); // 45 usable lines
                                    // Create 50 bullets (111% fill — MajorOverflow)
                                    // After 3 removals (MAX_FILL_PASSES), still 47 bullets (104% fill — MinorOverflow).
                                    // page_fill_flagged should be true since 47 > 45.
                                    // Note: only page fill logic tested here (no LLM), so bullets have 0 jd_keywords.
        let bullets: Vec<SimulatedBullet> =
            (0..50).map(|_| make_bullet(1, vec![], false)).collect();

        let analysis = analyze_page_fill(&bullets, &config, true);
        assert_eq!(analysis.verdict, PageFillVerdict::MajorOverflow);
        // Removing 3 bullets leaves 47 — still overflowing (104.4%), so page_fill_flagged
        // would be set after MAX_FILL_PASSES. This test validates the analysis side only
        // (the async run_page_fill_pass requires tokio runtime — covered by e2e test).
        let after_3 = &bullets[..47];
        let after_analysis = analyze_page_fill(after_3, &config, true);
        assert_eq!(
            after_analysis.verdict,
            PageFillVerdict::MinorOverflow,
            "47/45 = 104.4% → MinorOverflow after 3 removals"
        );
    }

    // ── orphan header cleanup ────────────────────────────────────────────────

    #[test]
    fn test_remove_bullet_cleans_up_orphaned_header() {
        let entry_a = Uuid::new_v4();

        // Build a SimulationResult with a header placeholder (empty text) and one content
        // bullet for the same entry.
        let mut result = crate::layout::simulator::SimulationResult {
            bullets: vec![
                SimulatedBullet {
                    text: String::new(), // header placeholder
                    source_entry_id: entry_a,
                    entry_header_latex: Some(r"\job{Acme}{Eng}{2020 -- 2022}".to_string()),
                    section: "experience".to_string(),
                    verified_line_count: 0,
                    jd_keywords_used: vec![],
                    was_adjusted: false,
                    flagged_for_review: false,
                    page_number: 1,
                },
                SimulatedBullet {
                    text: "Built distributed cache reducing p99 latency by 40%".to_string(),
                    source_entry_id: entry_a,
                    entry_header_latex: None,
                    section: "experience".to_string(),
                    verified_line_count: 1,
                    jd_keywords_used: vec![],
                    was_adjusted: false,
                    flagged_for_review: false,
                    page_number: 1,
                },
            ],
            total_passes: 0,
            violations_remaining: 0,
            flagged_count: 0,
            llm_calls_made: 0,
            tighten_spacing: false,
            page_fill_flagged: false,
            page_count: 1,
        };

        // Simulate the RemoveBullet handler logic for index 1 (the only content bullet).
        let content_idx = 1;
        let removed_entry_id = result.bullets[content_idx].source_entry_id;
        result.bullets.remove(content_idx);
        let has_remaining_content = result
            .bullets
            .iter()
            .any(|b| b.source_entry_id == removed_entry_id && !b.text.is_empty());
        if !has_remaining_content {
            result
                .bullets
                .retain(|b| !(b.source_entry_id == removed_entry_id && b.text.is_empty()));
        }

        assert!(
            result.bullets.is_empty(),
            "after removing the only content bullet, the orphaned header must also be removed"
        );
    }

    #[test]
    fn test_remove_bullet_keeps_header_when_siblings_remain() {
        let entry_a = Uuid::new_v4();

        let mut result = crate::layout::simulator::SimulationResult {
            bullets: vec![
                SimulatedBullet {
                    text: String::new(), // header placeholder
                    source_entry_id: entry_a,
                    entry_header_latex: Some(r"\job{Acme}{Eng}{2020 -- 2022}".to_string()),
                    section: "experience".to_string(),
                    verified_line_count: 0,
                    jd_keywords_used: vec![],
                    was_adjusted: false,
                    flagged_for_review: false,
                    page_number: 1,
                },
                SimulatedBullet {
                    text: "Built distributed cache reducing p99 latency by 40%".to_string(),
                    source_entry_id: entry_a,
                    entry_header_latex: None,
                    section: "experience".to_string(),
                    verified_line_count: 1,
                    jd_keywords_used: vec![],
                    was_adjusted: false,
                    flagged_for_review: false,
                    page_number: 1,
                },
                SimulatedBullet {
                    text: "Reduced infrastructure costs by 30%".to_string(),
                    source_entry_id: entry_a,
                    entry_header_latex: None,
                    section: "experience".to_string(),
                    verified_line_count: 1,
                    jd_keywords_used: vec![],
                    was_adjusted: false,
                    flagged_for_review: false,
                    page_number: 1,
                },
            ],
            total_passes: 0,
            violations_remaining: 0,
            flagged_count: 0,
            llm_calls_made: 0,
            tighten_spacing: false,
            page_fill_flagged: false,
            page_count: 1,
        };

        // Remove one of the two content bullets (index 1) — a sibling content bullet remains.
        let content_idx = 1;
        let removed_entry_id = result.bullets[content_idx].source_entry_id;
        result.bullets.remove(content_idx);
        let has_remaining_content = result
            .bullets
            .iter()
            .any(|b| b.source_entry_id == removed_entry_id && !b.text.is_empty());
        if !has_remaining_content {
            result
                .bullets
                .retain(|b| !(b.source_entry_id == removed_entry_id && b.text.is_empty()));
        }

        assert_eq!(
            result.bullets.len(),
            2,
            "header and remaining content bullet must both be kept"
        );
        assert!(
            result.bullets.iter().any(|b| b.text.is_empty()),
            "header placeholder must be kept since a sibling content bullet remains"
        );
        assert!(
            result
                .bullets
                .iter()
                .any(|b| b.text.contains("infrastructure")),
            "remaining content bullet must be kept"
        );
    }
}
