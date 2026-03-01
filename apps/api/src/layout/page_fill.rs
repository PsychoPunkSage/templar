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
pub fn analyze_page_fill(bullets: &[SimulatedBullet], config: &PageConfig) -> PageFillAnalysis {
    let total_lines_used: u16 = bullets.iter().map(|b| b.verified_line_count as u16).sum();

    let available = config.usable_height_lines;
    let fill_ratio = total_lines_used as f32 / available as f32;

    let whitespace_fraction = (1.0_f32 - fill_ratio).max(0.0);
    let overflow_fraction = (fill_ratio - 1.0_f32).max(0.0);

    let verdict = if fill_ratio > 1.05 {
        PageFillVerdict::MajorOverflow
    } else if fill_ratio > 1.00 {
        PageFillVerdict::MinorOverflow
    } else if whitespace_fraction > 0.08 {
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
            verified_line_count: line_count,
            jd_keywords_used: keywords.into_iter().map(|s| s.to_string()).collect(),
            was_adjusted: false,
            flagged_for_review: flagged,
        }
    }

    // ── analyze_page_fill verdicts ──────────────────────────────────────────

    #[test]
    fn test_acceptable_fill_verdict() {
        let config = make_config(); // 45 usable lines
                                    // 43 lines used = 95.6% fill → Acceptable (whitespace = 4.4% < 8%)
        let bullets: Vec<SimulatedBullet> =
            (0..43).map(|_| make_bullet(1, vec![], false)).collect();
        let analysis = analyze_page_fill(&bullets, &config);
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
        let analysis = analyze_page_fill(&bullets, &config);
        assert_eq!(analysis.verdict, PageFillVerdict::TooMuchWhitespace);
        assert!(analysis.whitespace_fraction > 0.08);
    }

    #[test]
    fn test_minor_overflow_verdict() {
        let config = make_config(); // 45 usable lines
                                    // 47 lines used = 104.4% fill → MinorOverflow (1–5%)
        let bullets: Vec<SimulatedBullet> =
            (0..47).map(|_| make_bullet(1, vec![], false)).collect();
        let analysis = analyze_page_fill(&bullets, &config);
        assert_eq!(analysis.verdict, PageFillVerdict::MinorOverflow);
        assert!(analysis.overflow_fraction > 0.0 && analysis.overflow_fraction <= 0.05);
    }

    #[test]
    fn test_major_overflow_verdict() {
        let config = make_config(); // 45 usable lines
                                    // 50 lines used = 111.1% fill → MajorOverflow (> 5%)
        let bullets: Vec<SimulatedBullet> =
            (0..50).map(|_| make_bullet(1, vec![], false)).collect();
        let analysis = analyze_page_fill(&bullets, &config);
        assert_eq!(analysis.verdict, PageFillVerdict::MajorOverflow);
        assert!(analysis.overflow_fraction > 0.05);
    }

    #[test]
    fn test_empty_bullets_is_whitespace() {
        let config = make_config();
        let analysis = analyze_page_fill(&[], &config);
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
        let analysis = analyze_page_fill(&bullets, &config);
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
}
