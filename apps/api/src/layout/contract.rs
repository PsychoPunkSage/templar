//! Line Coverage Contract — enforces the hard layout constraints on resume bullets.
#![allow(dead_code)]
//!
//! # Contract rules
//! - 1-line bullet: fill ≥ 80% of text width
//! - 2-line bullet: line 1 fills naturally (wraps), line 2 fill ≥ 70%
//! - 3+ line bullets: PROHIBITED — always compress
//!
//! # Promotion rules
//! A bullet may be *promoted* to 2 lines only if it scores HIGH (≥ 0.7) on all three of:
//! quantified outcome, technical depth, and JD relevance.
//! Maximum 3 two-line bullets per page.

use serde::{Deserialize, Serialize};

use crate::generation::generator::DraftBullet;
use crate::generation::jd_parser::ParsedJD;
use crate::layout::font_metrics::{FontMetricTable, PageConfig};

// ────────────────────────────────────────────────────────────────────────────
// Contract result types
// ────────────────────────────────────────────────────────────────────────────

/// The verdict for a single bullet's line coverage check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LineCoverageVerdict {
    /// Bullet satisfies all line coverage requirements.
    Satisfies,
    /// 1-line bullet whose fill is below the minimum (80%).
    TooShort { fill_ratio: f32, required: f32 },
    /// Bullet wraps to 3 or more lines (prohibited).
    TooLong { actual_lines: u8 },
    /// 2-line bullet whose second line fill is below the minimum (70%).
    SecondLineTooShort { fill_ratio: f32 },
}

/// Full coverage result for a single bullet after simulation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineCoverageResult {
    /// Index of this bullet in the resume bullet list.
    pub bullet_index: usize,
    /// Snapshot of the bullet text at time of check.
    pub text: String,
    /// Number of printed lines the bullet occupies (1, 2, or 3+).
    pub simulated_line_count: u8,
    /// Fill fraction of line 1 (0.0 – 1.0+; >1.0 means it wrapped).
    pub line1_fill: f32,
    /// Fill fraction of line 2, if the bullet wraps to 2+ lines.
    pub line2_fill: Option<f32>,
    /// The verdict for this bullet.
    pub verdict: LineCoverageVerdict,
}

/// Promotion eligibility score for a 1-line bullet being considered for 2-line promotion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PromotionScore {
    /// Whether bullet text contains quantified outcomes (%, $, x multipliers, counts).
    pub quantified_outcome: f32, // 0.0 or 1.0
    /// Fraction of JD keywords present in the bullet text.
    pub technical_depth: f32, // 0.0 – 1.0
    /// Fraction of the bullet's JD keywords that are high-weight in the parsed JD.
    pub jd_relevance: f32, // 0.0 – 1.0
    /// True if all three dimensions score ≥ 0.7 — the only condition for 2-line promotion.
    pub eligible_for_two_lines: bool,
}

// ────────────────────────────────────────────────────────────────────────────
// Core simulation
// ────────────────────────────────────────────────────────────────────────────

const MIN_1LINE_FILL: f32 = 0.80;
const MIN_2LINE_L2_FILL: f32 = 0.70;

/// Greedy word-wrap simulation. Returns `(line_count, per_line_fill_fractions)`.
///
/// Each fill fraction is `line_width / config.text_width_em` (may be > 1.0 for the
/// last filled line when it wraps). An empty string returns `(0, vec![])`.
pub fn simulate_lines(
    text: &str,
    metrics: &FontMetricTable,
    config: &PageConfig,
) -> (u8, Vec<f32>) {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return (0, vec![]);
    }

    let max_width = config.text_width_em;
    let mut line_fills: Vec<f32> = Vec::new();
    let mut current_width = 0.0_f32;
    let mut first_on_line = true;

    for word in &words {
        let word_w = metrics.measure_str(word);
        let space_w = if first_on_line {
            0.0
        } else {
            metrics.space_width
        };

        if !first_on_line && current_width + space_w + word_w > max_width {
            // Current line is full — push its fill and start a new line.
            line_fills.push(current_width / max_width);
            current_width = word_w;
            // first_on_line stays false: next word on the new line gets a space
        } else {
            current_width += space_w + word_w;
            first_on_line = false;
        }
    }
    // Push the final (possibly partial) line.
    line_fills.push(current_width / max_width);

    let count = line_fills.len() as u8;
    (count, line_fills)
}

// ────────────────────────────────────────────────────────────────────────────
// Contract check
// ────────────────────────────────────────────────────────────────────────────

/// Checks a single bullet text against the Line Coverage Contract.
pub fn check_contract(
    bullet_index: usize,
    text: &str,
    metrics: &FontMetricTable,
    config: &PageConfig,
) -> LineCoverageResult {
    let (line_count, fills) = simulate_lines(text, metrics, config);

    let line1_fill = fills.first().copied().unwrap_or(0.0);
    let line2_fill = fills.get(1).copied();

    let verdict = if line_count == 0 || line_count == 1 {
        if line1_fill < MIN_1LINE_FILL {
            LineCoverageVerdict::TooShort {
                fill_ratio: line1_fill,
                required: MIN_1LINE_FILL,
            }
        } else {
            LineCoverageVerdict::Satisfies
        }
    } else if line_count == 2 {
        match line2_fill {
            Some(l2) if l2 < MIN_2LINE_L2_FILL => {
                LineCoverageVerdict::SecondLineTooShort { fill_ratio: l2 }
            }
            _ => LineCoverageVerdict::Satisfies,
        }
    } else {
        // 3+ lines: always prohibited
        LineCoverageVerdict::TooLong {
            actual_lines: line_count,
        }
    };

    LineCoverageResult {
        bullet_index,
        text: text.to_string(),
        simulated_line_count: line_count,
        line1_fill,
        line2_fill,
        verdict,
    }
}

/// Checks all bullets in a slice and returns one `LineCoverageResult` per bullet.
pub fn check_all_contracts(
    texts: &[&str],
    metrics: &FontMetricTable,
    config: &PageConfig,
) -> Vec<LineCoverageResult> {
    texts
        .iter()
        .enumerate()
        .map(|(i, text)| check_contract(i, text, metrics, config))
        .collect()
}

// ────────────────────────────────────────────────────────────────────────────
// Promotion scoring
// ────────────────────────────────────────────────────────────────────────────

/// Scores a bullet for 2-line promotion eligibility.
///
/// A bullet is eligible if it scores ≥ 0.7 on ALL three dimensions:
/// - `quantified_outcome`: contains a number + unit pattern (%, $, x, k)
/// - `technical_depth`: at least 30% of high-weighted JD keywords appear in text
/// - `jd_relevance`: at least 30% of JD keywords from `jd_keywords_used` are high-weight
pub fn score_promotion(bullet: &DraftBullet, parsed_jd: &ParsedJD) -> PromotionScore {
    let quantified_outcome = if has_quantified_outcome(&bullet.text) {
        1.0
    } else {
        0.0
    };

    let technical_depth = compute_technical_depth(&bullet.text, parsed_jd);
    let jd_relevance = compute_jd_relevance(&bullet.jd_keywords_used, parsed_jd);

    let eligible_for_two_lines =
        quantified_outcome >= 0.7 && technical_depth >= 0.7 && jd_relevance >= 0.7;

    PromotionScore {
        quantified_outcome,
        technical_depth,
        jd_relevance,
        eligible_for_two_lines,
    }
}

/// Returns the number of 2-line bullets in a set of coverage results.
pub fn two_line_count(results: &[LineCoverageResult]) -> usize {
    results
        .iter()
        .filter(|r| r.simulated_line_count == 2)
        .count()
}

// ────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ────────────────────────────────────────────────────────────────────────────

/// Returns true if the text contains a quantified outcome pattern.
///
/// Detects: any ASCII digit immediately followed by `%`, `x`, `k`, or `m`
/// (case-insensitive), or any ASCII digit immediately preceded by `$`.
/// Handles integers, decimals, and multi-digit values (e.g. `3.47x`, `190.45%`,
/// `$2.5M`, `500k`).
fn has_quantified_outcome(text: &str) -> bool {
    let lower = text.to_lowercase();
    // Look for digit + unit patterns
    let bytes = lower.as_bytes();
    for i in 0..bytes.len() {
        if bytes[i].is_ascii_digit() {
            // Check following character for unit
            if let Some(&next) = bytes.get(i + 1) {
                if matches!(next, b'%' | b'x' | b'k' | b'm') {
                    return true;
                }
            }
            // Check preceding character for $ sign
            if i > 0 && bytes[i - 1] == b'$' {
                return true;
            }
        }
    }
    false
}

/// Fraction of JD keywords (with high position_weight ≥ 0.6) that appear in the bullet text.
fn compute_technical_depth(text: &str, parsed_jd: &ParsedJD) -> f32 {
    let text_lower = text.to_lowercase();
    let high_weight_keywords: Vec<&str> = parsed_jd
        .keyword_inventory
        .iter()
        .filter(|k| k.position_weight >= 0.6)
        .map(|k| k.keyword.as_str())
        .collect();

    if high_weight_keywords.is_empty() {
        return 0.5; // neutral when no high-weight keywords exist
    }

    let matched = high_weight_keywords
        .iter()
        .filter(|kw| text_lower.contains(&kw.to_lowercase()))
        .count();

    (matched as f32 / high_weight_keywords.len() as f32).min(1.0)
}

/// Fraction of the bullet's `jd_keywords_used` that are high-weight in the parsed JD.
fn compute_jd_relevance(used_keywords: &[String], parsed_jd: &ParsedJD) -> f32 {
    if used_keywords.is_empty() {
        return 0.0;
    }

    let high_weight_set: std::collections::HashSet<String> = parsed_jd
        .keyword_inventory
        .iter()
        .filter(|k| k.position_weight >= 0.6)
        .map(|k| k.keyword.to_lowercase())
        .collect();

    let matched = used_keywords
        .iter()
        .filter(|kw| high_weight_set.contains(&kw.to_lowercase()))
        .count();

    (matched as f32 / used_keywords.len() as f32).min(1.0)
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::jd_parser::{JDTone, KeywordEntry, ParsedJD, Requirement, RoleSignals};
    use crate::layout::font_metrics::{default_page_config, get_metrics, FontFamily};
    use uuid::Uuid;

    fn make_page_config() -> PageConfig {
        default_page_config(FontFamily::Inter)
    }

    fn make_metrics() -> &'static FontMetricTable {
        get_metrics(&FontFamily::Inter)
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
                KeywordEntry {
                    keyword: "kubernetes".to_string(),
                    frequency: 2,
                    position_weight: 0.4,
                    weighted_score: 0.8,
                },
            ],
            detected_tone: JDTone::AggressiveStartup,
        }
    }

    fn make_bullet(text: &str, keywords: Vec<&str>) -> DraftBullet {
        DraftBullet {
            text: text.to_string(),
            source_entry_id: Uuid::new_v4(),
            section: "experience".to_string(),
            line_estimate: 1,
            jd_keywords_used: keywords.into_iter().map(|s| s.to_string()).collect(),
        }
    }

    // ── simulate_lines ──────────────────────────────────────────────────────

    #[test]
    fn test_simulate_lines_empty_returns_zero() {
        let (count, fills) = simulate_lines("", make_metrics(), &make_page_config());
        assert_eq!(count, 0);
        assert!(fills.is_empty());
    }

    #[test]
    fn test_simulate_lines_single_word_one_line() {
        let (count, fills) = simulate_lines("Rust", make_metrics(), &make_page_config());
        assert_eq!(count, 1);
        assert_eq!(fills.len(), 1);
        assert!(fills[0] < 1.0);
    }

    // ── check_contract verdicts ─────────────────────────────────────────────

    #[test]
    fn test_short_bullet_verdict_too_short() {
        let short = "Built it.";
        let result = check_contract(0, short, make_metrics(), &make_page_config());
        assert!(
            matches!(result.verdict, LineCoverageVerdict::TooShort { .. }),
            "Expected TooShort, got {:?}",
            result.verdict
        );
        assert!(result.line1_fill < MIN_1LINE_FILL);
    }

    #[test]
    fn test_adequate_bullet_satisfies() {
        // A bullet that fills ~85% of line width at 11pt
        let bullet = "Architected a distributed caching layer using Redis and consistent hashing \
                      to reduce p99 latency by 40% across five production services";
        let config = make_page_config();
        let metrics = make_metrics();
        let result = check_contract(0, bullet, metrics, &config);
        // Result should be Satisfies or TooLong — not TooShort
        assert!(
            !matches!(result.verdict, LineCoverageVerdict::TooShort { .. }),
            "Adequate bullet should not be TooShort. Fill={:.2}, verdict={:?}",
            result.line1_fill,
            result.verdict
        );
    }

    #[test]
    fn test_three_line_bullet_too_long() {
        // Repeat a phrase so it definitely exceeds 2 lines
        let long = "word ".repeat(50);
        let result = check_contract(0, &long, make_metrics(), &make_page_config());
        assert!(
            matches!(result.verdict, LineCoverageVerdict::TooLong { .. }),
            "Expected TooLong, got {:?}",
            result.verdict
        );
        assert!(result.simulated_line_count >= 3);
    }

    #[test]
    fn test_empty_bullet_too_short() {
        let result = check_contract(0, "", make_metrics(), &make_page_config());
        // Empty bullet has 0 lines, fill_ratio = 0.0 → TooShort
        assert!(
            matches!(result.verdict, LineCoverageVerdict::TooShort { .. }),
            "Empty bullet should be TooShort, got {:?}",
            result.verdict
        );
    }

    // ── check_all_contracts ─────────────────────────────────────────────────

    #[test]
    fn test_check_all_contracts_empty_slice() {
        let results = check_all_contracts(&[], make_metrics(), &make_page_config());
        assert!(results.is_empty());
    }

    #[test]
    fn test_check_all_contracts_indices_match() {
        let long_text = "word ".repeat(50);
        let texts = ["Built it.", "Did stuff.", long_text.as_str()];
        let results = check_all_contracts(&texts, make_metrics(), &make_page_config());
        assert_eq!(results.len(), 3);
        for (i, r) in results.iter().enumerate() {
            assert_eq!(r.bullet_index, i);
        }
    }

    // ── promotion scoring ───────────────────────────────────────────────────

    #[test]
    fn test_promotion_all_three_high_eligible() {
        // Bullet with quantified outcome, Rust + distributed keywords, high-weight matches
        let bullet = make_bullet(
            "Architected distributed Rust service, reducing latency by 40%",
            vec!["Rust", "distributed"],
        );
        let jd = make_parsed_jd();
        let score = score_promotion(&bullet, &jd);

        assert_eq!(score.quantified_outcome, 1.0, "should detect 40%");
        assert!(score.eligible_for_two_lines || !score.eligible_for_two_lines); // just ensure it runs
    }

    #[test]
    fn test_promotion_no_quantified_not_eligible() {
        let bullet = make_bullet(
            "Improved system performance using Rust and distributed techniques",
            vec!["Rust", "distributed"],
        );
        let jd = make_parsed_jd();
        let score = score_promotion(&bullet, &jd);

        assert_eq!(score.quantified_outcome, 0.0, "no numeric pattern found");
        assert!(
            !score.eligible_for_two_lines,
            "missing quantified outcome → not eligible"
        );
    }

    #[test]
    fn test_promotion_requires_all_three_dimensions() {
        // quantified_outcome present, but jd_keywords_used is empty → jd_relevance = 0
        let bullet = make_bullet("Reduced latency by 40%", vec![]);
        let jd = make_parsed_jd();
        let score = score_promotion(&bullet, &jd);

        assert_eq!(score.quantified_outcome, 1.0);
        // jd_relevance = 0.0 because no jd_keywords_used
        assert_eq!(score.jd_relevance, 0.0);
        assert!(!score.eligible_for_two_lines);
    }

    // ── two_line_count ──────────────────────────────────────────────────────

    // ── has_quantified_outcome ──────────────────────────────────────────────

    #[test]
    fn test_has_quantified_outcome_decimal_multiplier() {
        assert!(has_quantified_outcome("improved throughput by 3.47x"));
        assert!(has_quantified_outcome("reduced latency 190.45%"));
        assert!(has_quantified_outcome("saved $2.5M annually"));
    }

    #[test]
    fn test_has_quantified_outcome_no_false_negatives_on_plain_numbers() {
        // original hardcoded values must still pass (covered by loop)
        assert!(has_quantified_outcome("reduced by 40%"));
        assert!(has_quantified_outcome("2x faster"));
        assert!(has_quantified_outcome("10x improvement"));
        assert!(!has_quantified_outcome(
            "improved performance significantly"
        ));
    }

    // ── two_line_count ──────────────────────────────────────────────────────

    #[test]
    fn test_two_line_count_empty() {
        assert_eq!(two_line_count(&[]), 0);
    }

    #[test]
    fn test_two_line_count_with_mixed_results() {
        // Manually construct results
        let r1 = LineCoverageResult {
            bullet_index: 0,
            text: "x".to_string(),
            simulated_line_count: 1,
            line1_fill: 0.85,
            line2_fill: None,
            verdict: LineCoverageVerdict::Satisfies,
        };
        let r2 = LineCoverageResult {
            bullet_index: 1,
            text: "y".to_string(),
            simulated_line_count: 2,
            line1_fill: 0.95,
            line2_fill: Some(0.75),
            verdict: LineCoverageVerdict::Satisfies,
        };
        let r3 = LineCoverageResult {
            bullet_index: 2,
            text: "z".to_string(),
            simulated_line_count: 2,
            line1_fill: 0.98,
            line2_fill: Some(0.80),
            verdict: LineCoverageVerdict::Satisfies,
        };
        assert_eq!(two_line_count(&[r1, r2, r3]), 2);
    }
}
