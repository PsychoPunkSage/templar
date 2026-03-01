#![allow(dead_code)]

//! Fit Scoring — pluggable, trait-based scorer that measures user context vs a parsed JD.
//!
//! Default: `KeywordFitScorer` (pure-Rust, fast, deterministic, fully testable).
//! Future: `LlmFitScorer` (semantic via Claude — stubbed for Phase 7).
//!
//! `AppState` holds an `Arc<dyn FitScorer>`, swapped at startup via config.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::errors::AppError;
use crate::generation::jd_parser::ParsedJD;
use crate::llm_client::LlmClient;
use crate::models::context::ContextEntryRow;

// ────────────────────────────────────────────────────────────────────────────
// Output data models (shared across all scorer backends)
// ────────────────────────────────────────────────────────────────────────────

/// A single matched dimension between user context and a JD keyword/requirement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitMatch {
    pub dimension: String,
    pub context_evidence: String, // which entry covers it
    pub jd_requirement: String,
    pub strength: f32, // 0.0 – 1.0
}

/// A JD keyword or requirement not covered by any context entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Gap {
    pub keyword: String,
    pub jd_frequency: u32,
    pub suggestion: Option<String>, // closest context entry_id, if any
}

/// Full fit report returned to callers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FitReport {
    pub overall_score: u32,              // 0 – 100
    pub strong_matches: Vec<FitMatch>,   // strength ≥ 0.8
    pub partial_matches: Vec<FitMatch>,  // 0.4 – 0.79
    pub gaps: Vec<Gap>,                  // strength < 0.4
    pub recommendation: String,
    pub scorer_backend: String, // "keyword" | "llm" — for transparency
}

// ────────────────────────────────────────────────────────────────────────────
// Trait definition
// ────────────────────────────────────────────────────────────────────────────

/// The fit scorer trait. Implement this to swap backends without touching
/// the endpoint, handler, or caller code.
///
/// Carried in `AppState` as `Arc<dyn FitScorer>`.
#[async_trait]
pub trait FitScorer: Send + Sync {
    async fn score(
        &self,
        entries: &[ContextEntryRow],
        parsed_jd: &ParsedJD,
    ) -> Result<FitReport, AppError>;
}

// ────────────────────────────────────────────────────────────────────────────
// KeywordFitScorer — default Phase 2 implementation
// ────────────────────────────────────────────────────────────────────────────

/// Pure-Rust keyword-based fit scorer. Fast, deterministic, no LLM call.
///
/// Algorithm:
/// 1. For each keyword in ParsedJD.keyword_inventory:
///    - tag exact match → strength 1.0
///    - raw_text substring match → strength 0.6
///    - no match → strength 0.0
/// 2. overall_score = Σ(strength × weighted_score) / Σ(weighted_score) × 100
/// 3. Classify: strong (≥0.8), partial (0.4–0.79), gap (<0.4)
pub struct KeywordFitScorer;

#[async_trait]
impl FitScorer for KeywordFitScorer {
    async fn score(
        &self,
        entries: &[ContextEntryRow],
        parsed_jd: &ParsedJD,
    ) -> Result<FitReport, AppError> {
        compute_keyword_fit(entries, parsed_jd)
    }
}

// ────────────────────────────────────────────────────────────────────────────
// LlmFitScorer — semantic scorer stub (Phase 7)
// ────────────────────────────────────────────────────────────────────────────

/// Semantic fit scorer via Claude. Compile but not default in Phase 2.
pub struct LlmFitScorer(pub LlmClient);

#[async_trait]
impl FitScorer for LlmFitScorer {
    async fn score(
        &self,
        _entries: &[ContextEntryRow],
        _parsed_jd: &ParsedJD,
    ) -> Result<FitReport, AppError> {
        // TODO Phase 7 Intelligence Layer: semantic scoring via LLM.
        // Call llm.call_json::<FitReport>() with entries + JD as context.
        todo!("LLM fit scorer — Phase 7 Intelligence Layer")
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Core keyword fit algorithm
// ────────────────────────────────────────────────────────────────────────────

fn compute_keyword_fit(
    entries: &[ContextEntryRow],
    parsed_jd: &ParsedJD,
) -> Result<FitReport, AppError> {
    let keywords = &parsed_jd.keyword_inventory;

    if keywords.is_empty() {
        return Ok(FitReport {
            overall_score: 0,
            strong_matches: vec![],
            partial_matches: vec![],
            gaps: vec![],
            recommendation: "No keywords found in JD — cannot score fit.".to_string(),
            scorer_backend: "keyword".to_string(),
        });
    }

    let mut strong_matches = Vec::new();
    let mut partial_matches = Vec::new();
    let mut gaps = Vec::new();

    let mut total_weighted = 0.0_f32;
    let mut total_score = 0.0_f32;

    for kw_entry in keywords {
        let keyword_lower = kw_entry.keyword.to_lowercase();
        total_weighted += kw_entry.weighted_score;

        // Find the best-matching context entry for this keyword
        let mut best_strength = 0.0_f32;
        let mut best_evidence = String::new();

        for entry in entries {
            // Tag exact match → 1.0
            let tag_match = entry
                .tags
                .iter()
                .any(|t| t.to_lowercase() == keyword_lower);

            // raw_text substring match → 0.6
            let text_match = entry
                .raw_text
                .as_deref()
                .map(|t| t.to_lowercase().contains(&keyword_lower))
                .unwrap_or(false);

            let strength = if tag_match {
                1.0
            } else if text_match {
                0.6
            } else {
                0.0
            };

            if strength > best_strength {
                best_strength = strength;
                best_evidence = format!("entry {} ({})", entry.entry_id, entry.entry_type);
            }
        }

        total_score += best_strength * kw_entry.weighted_score;

        let fit_match = FitMatch {
            dimension: kw_entry.keyword.clone(),
            context_evidence: best_evidence,
            jd_requirement: kw_entry.keyword.clone(),
            strength: best_strength,
        };

        if best_strength >= 0.8 {
            strong_matches.push(fit_match);
        } else if best_strength >= 0.4 {
            partial_matches.push(fit_match);
        } else {
            let suggestion = find_closest_entry(entries, &keyword_lower);
            gaps.push(Gap {
                keyword: kw_entry.keyword.clone(),
                jd_frequency: kw_entry.frequency,
                suggestion,
            });
        }
    }

    let overall_score = if total_weighted > 0.0 {
        ((total_score / total_weighted) * 100.0).round() as u32
    } else {
        0
    };

    let recommendation = build_recommendation(overall_score, &gaps);

    Ok(FitReport {
        overall_score,
        strong_matches,
        partial_matches,
        gaps,
        recommendation,
        scorer_backend: "keyword".to_string(),
    })
}

/// Finds the entry whose tags most closely overlap with the keyword (for gap suggestions).
fn find_closest_entry(entries: &[ContextEntryRow], keyword: &str) -> Option<String> {
    for entry in entries {
        for tag in &entry.tags {
            let tag_lower = tag.to_lowercase();
            if tag_lower.contains(keyword) || keyword.contains(&tag_lower) {
                return Some(entry.entry_id.to_string());
            }
        }
    }
    None
}

/// Builds a human-readable recommendation string from score and gaps.
fn build_recommendation(score: u32, gaps: &[Gap]) -> String {
    let top_gaps: Vec<&str> = gaps.iter().take(3).map(|g| g.keyword.as_str()).collect();

    if score >= 80 {
        "Strong fit. Your context directly covers the key JD requirements.".to_string()
    } else if score >= 60 {
        format!(
            "Moderate fit ({score}/100). Consider adding context for: {}.",
            top_gaps.join(", ")
        )
    } else {
        format!(
            "Low fit ({score}/100). Significant gaps: {}. Consider whether to tailor your context or apply.",
            top_gaps.join(", ")
        )
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::jd_parser::{JDTone, KeywordEntry, ParsedJD, Requirement, RoleSignals};
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    fn make_entry(
        entry_id: Uuid,
        tags: Vec<String>,
        raw_text: Option<String>,
    ) -> ContextEntryRow {
        ContextEntryRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_id,
            version: 1,
            entry_type: "experience".to_string(),
            data: json!({}),
            raw_text,
            recency_score: 1.0,
            impact_score: 0.8,
            tags,
            flagged_evergreen: false,
            contribution_type: "primary_contributor".to_string(),
            created_at: Utc::now(),
        }
    }

    fn make_parsed_jd(keywords: Vec<(&str, u32, f32)>) -> ParsedJD {
        ParsedJD {
            hard_requirements: vec![Requirement {
                text: "Rust programming".to_string(),
                is_required: true,
            }],
            soft_signals: vec![],
            role_signals: RoleSignals {
                is_startup: false,
                is_ic_focused: true,
                is_research: false,
                seniority: "senior".to_string(),
            },
            keyword_inventory: keywords
                .into_iter()
                .map(|(kw, freq, pw)| KeywordEntry {
                    keyword: kw.to_string(),
                    frequency: freq,
                    position_weight: pw,
                    weighted_score: freq as f32 * pw,
                })
                .collect(),
            detected_tone: JDTone::CollaborativeEnterprise,
        }
    }

    #[test]
    fn test_perfect_tag_match_scores_strong() {
        let entry_id = Uuid::new_v4();
        let entries = vec![make_entry(
            entry_id,
            vec!["rust".to_string(), "distributed-systems".to_string()],
            None,
        )];
        let parsed_jd = make_parsed_jd(vec![("rust", 5, 0.8), ("distributed-systems", 3, 0.6)]);

        let report = compute_keyword_fit(&entries, &parsed_jd).unwrap();
        assert!(
            report.overall_score >= 80,
            "Expected ≥80, got {}",
            report.overall_score
        );
        assert_eq!(report.strong_matches.len(), 2);
        assert!(report.gaps.is_empty());
    }

    #[test]
    fn test_text_match_scores_partial() {
        let entry_id = Uuid::new_v4();
        let entries = vec![make_entry(
            entry_id,
            vec![],
            Some("I have extensive Kubernetes deployment experience".to_string()),
        )];
        let parsed_jd = make_parsed_jd(vec![("kubernetes", 3, 0.8)]);

        let report = compute_keyword_fit(&entries, &parsed_jd).unwrap();
        // text match = 0.6 strength → partial (0.4–0.79)
        assert_eq!(report.partial_matches.len(), 1);
        assert_eq!(report.strong_matches.len(), 0);
        assert_eq!(report.gaps.len(), 0);
    }

    #[test]
    fn test_no_match_creates_gap() {
        let entries = vec![make_entry(
            Uuid::new_v4(),
            vec!["python".to_string()],
            None,
        )];
        let parsed_jd = make_parsed_jd(vec![("rust", 5, 0.8)]);

        let report = compute_keyword_fit(&entries, &parsed_jd).unwrap();
        assert_eq!(report.gaps.len(), 1);
        assert_eq!(report.gaps[0].keyword, "rust");
        assert_eq!(report.gaps[0].jd_frequency, 5);
    }

    #[test]
    fn test_empty_keywords_returns_zero_score() {
        let entries = vec![make_entry(Uuid::new_v4(), vec![], None)];
        let parsed_jd = make_parsed_jd(vec![]);

        let report = compute_keyword_fit(&entries, &parsed_jd).unwrap();
        assert_eq!(report.overall_score, 0);
        assert!(report.strong_matches.is_empty());
        assert!(report.gaps.is_empty());
    }

    #[test]
    fn test_overall_score_bounded_0_to_100() {
        let entries = vec![make_entry(
            Uuid::new_v4(),
            vec!["rust".to_string()],
            None,
        )];
        let parsed_jd = make_parsed_jd(vec![("rust", 10, 1.0), ("java", 1, 0.1)]);

        let report = compute_keyword_fit(&entries, &parsed_jd).unwrap();
        assert!(report.overall_score <= 100);
    }

    #[test]
    fn test_scorer_backend_label_is_keyword() {
        let report = compute_keyword_fit(&[], &make_parsed_jd(vec![])).unwrap();
        assert_eq!(report.scorer_backend, "keyword");
    }

    #[test]
    fn test_strong_match_threshold_is_0_8() {
        // Tag match = 1.0 strength → strong_matches
        let entries = vec![make_entry(
            Uuid::new_v4(),
            vec!["rust".to_string()],
            None,
        )];
        let parsed_jd = make_parsed_jd(vec![("rust", 1, 0.8)]);
        let report = compute_keyword_fit(&entries, &parsed_jd).unwrap();
        assert_eq!(report.strong_matches.len(), 1);
        assert_eq!(report.strong_matches[0].strength, 1.0);
    }

    #[test]
    fn test_recommendation_high_score() {
        let rec = build_recommendation(85, &[]);
        assert!(rec.contains("Strong fit"));
    }

    #[test]
    fn test_recommendation_moderate_score_lists_gaps() {
        let gaps = vec![Gap {
            keyword: "Kafka".to_string(),
            jd_frequency: 3,
            suggestion: None,
        }];
        let rec = build_recommendation(65, &gaps);
        assert!(rec.contains("Kafka"));
        assert!(rec.contains("65"));
    }

    #[test]
    fn test_recommendation_low_score() {
        let gaps = vec![Gap {
            keyword: "Rust".to_string(),
            jd_frequency: 5,
            suggestion: None,
        }];
        let rec = build_recommendation(30, &gaps);
        assert!(rec.contains("30"));
        assert!(rec.contains("Rust"));
    }
}
