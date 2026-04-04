#![allow(dead_code)]

//! Fit Scoring — pluggable, trait-based scorer measuring candidate context vs a parsed JD.
//!
//! Two implementations:
//! - [`KeywordFitScorer`]: pure-Rust keyword matching. Fast, deterministic, used in generation.
//! - [`LlmFitScorer`]: semantic scoring via Claude. Used by the explicit fit-score endpoint.
//!
//! Prompt construction helpers (private):
//! - `build_entries_summary()`: per-entry metadata + raw_text snippet (≤500 chars). Gives
//!   Claude real evidence of what the candidate did — not just a label.
//! - `build_jd_role_context()`: role shape signals from ParsedJD (seniority, culture, tone,
//!   soft signals). Fills {jd_text} without duplicating {jd_requirements}.
//! - `extract_raw_text_snippet()`: extracts top N non-trivial lines, hard-capped at 500 chars.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
    pub overall_score: u32,             // 0 – 100
    pub strong_matches: Vec<FitMatch>,  // strength ≥ 0.8
    pub partial_matches: Vec<FitMatch>, // 0.4 – 0.79
    pub gaps: Vec<Gap>,                 // strength < 0.4
    pub recommendation: String,
    pub scorer_backend: String, // "keyword" | "llm" — for transparency
    /// Entry IDs selected by the LLM fit scorer as relevant to this JD.
    /// Empty when KeywordFitScorer is used — falls back to all SelectionResult entries.
    #[serde(default)]
    pub selected_entry_ids: Vec<Uuid>,
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

/// Semantic fit scorer via Claude (Phase 7.0 implementation).
pub struct LlmFitScorer(pub LlmClient);

/// Intermediate response type matching the LLM output schema.
#[derive(Debug, Deserialize)]
struct LlmFitScoreResponse {
    overall_score: u32,
    strong_matches: Vec<FitMatch>,
    partial_matches: Vec<FitMatch>,
    gaps: Vec<Gap>,
    recommendation: String,
    #[serde(default)]
    selected_entry_ids: Vec<String>,
}

#[async_trait]
impl FitScorer for LlmFitScorer {
    async fn score(
        &self,
        entries: &[ContextEntryRow],
        parsed_jd: &ParsedJD,
    ) -> Result<FitReport, AppError> {
        use crate::generation::prompts::{LLM_FIT_SCORE_PROMPT_TEMPLATE, LLM_FIT_SCORE_SYSTEM};

        // Build entries summary (compact representation for the LLM)
        let entries_summary = build_entries_summary(entries);

        // Build JD keywords string
        let jd_keywords = parsed_jd
            .keyword_inventory
            .iter()
            .map(|k| {
                format!(
                    "{} (freq={}, weight={:.1})",
                    k.keyword, k.frequency, k.position_weight
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Build JD requirements string
        let jd_requirements = parsed_jd
            .hard_requirements
            .iter()
            .map(|r| {
                format!(
                    "- [{}] {}",
                    if r.is_required {
                        "REQUIRED"
                    } else {
                        "preferred"
                    },
                    r.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = LLM_FIT_SCORE_PROMPT_TEMPLATE
            .replace("{entries_summary}", &entries_summary)
            .replace("{jd_keywords}", &jd_keywords)
            .replace("{jd_requirements}", &jd_requirements)
            .replace("{jd_text}", &build_jd_role_context(parsed_jd));

        match self
            .0
            .call_json::<LlmFitScoreResponse>(&prompt, LLM_FIT_SCORE_SYSTEM)
            .await
        {
            Ok(resp) => {
                let selected_entry_ids: Vec<Uuid> = resp
                    .selected_entry_ids
                    .iter()
                    .filter_map(|s| {
                        Uuid::parse_str(s).map_err(|e| {
                            tracing::warn!(
                                raw_id = %s,
                                error = %e,
                                "LlmFitScorer: failed to parse selected_entry_id UUID — skipping"
                            );
                        }).ok()
                    })
                    .collect();
                Ok(FitReport {
                    overall_score: resp.overall_score.clamp(0, 100),
                    strong_matches: resp.strong_matches,
                    partial_matches: resp.partial_matches,
                    gaps: resp.gaps,
                    recommendation: resp.recommendation,
                    scorer_backend: "llm".to_string(),
                    selected_entry_ids,
                })
            }
            Err(e) => {
                // Fall back to keyword scorer on LLM error
                tracing::warn!(error = %e, "LlmFitScorer: LLM call failed, falling back to keyword scorer");
                let mut report = compute_keyword_fit(entries, parsed_jd)?;
                report.scorer_backend = "keyword_fallback".to_string();
                Ok(report)
            }
        }
    }
}

impl LlmFitScorer {
    /// Diagnostic variant: passes the full untruncated raw_text per entry and the complete
    /// original JD text as-is to Claude. Use this to verify score variation is working.
    /// Once confirmed, switch back to score() which uses the token-optimized path.
    pub async fn score_full(
        &self,
        entries: &[ContextEntryRow],
        parsed_jd: &ParsedJD,
        raw_jd_text: &str,
    ) -> Result<FitReport, AppError> {
        use crate::generation::prompts::{LLM_FIT_SCORE_PROMPT_TEMPLATE, LLM_FIT_SCORE_SYSTEM};

        let entries_summary = build_entries_summary_full(entries);

        let jd_keywords = parsed_jd
            .keyword_inventory
            .iter()
            .map(|k| {
                format!(
                    "{} (freq={}, weight={:.1})",
                    k.keyword, k.frequency, k.position_weight
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let jd_requirements = parsed_jd
            .hard_requirements
            .iter()
            .map(|r| {
                format!(
                    "- [{}] {}",
                    if r.is_required {
                        "REQUIRED"
                    } else {
                        "preferred"
                    },
                    r.text
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        let prompt = LLM_FIT_SCORE_PROMPT_TEMPLATE
            .replace("{entries_summary}", &entries_summary)
            .replace("{jd_keywords}", &jd_keywords)
            .replace("{jd_requirements}", &jd_requirements)
            .replace("{jd_text}", raw_jd_text);

        match self
            .0
            .call_json::<LlmFitScoreResponse>(&prompt, LLM_FIT_SCORE_SYSTEM)
            .await
        {
            Ok(resp) => {
                let selected_entry_ids: Vec<Uuid> = resp
                    .selected_entry_ids
                    .iter()
                    .filter_map(|s| {
                        Uuid::parse_str(s).map_err(|e| {
                            tracing::warn!(
                                raw_id = %s,
                                error = %e,
                                "LlmFitScorer::score_full: failed to parse selected_entry_id UUID — skipping"
                            );
                        }).ok()
                    })
                    .collect();
                Ok(FitReport {
                    overall_score: resp.overall_score.clamp(0, 100),
                    strong_matches: resp.strong_matches,
                    partial_matches: resp.partial_matches,
                    gaps: resp.gaps,
                    recommendation: resp.recommendation,
                    scorer_backend: "llm_full".to_string(),
                    selected_entry_ids,
                })
            }
            Err(e) => {
                tracing::warn!(error = %e, "LlmFitScorer::score_full: LLM call failed, falling back to keyword scorer");
                let mut report = compute_keyword_fit(entries, parsed_jd)?;
                report.scorer_backend = "keyword_fallback".to_string();
                Ok(report)
            }
        }
    }
}

/// Extracts up to `max_lines` meaningful lines from a raw context entry for LLM prompts.
/// Lines shorter than 10 chars (headers, dividers) are skipped.
/// Falls back to first 500 chars if no line structure is found.
/// Keeps token cost bounded regardless of entry length.
fn extract_raw_text_snippet(raw_text: &str, max_lines: usize) -> String {
    const MAX_CHARS: usize = 500;

    let lines: Vec<&str> = raw_text
        .lines()
        .map(str::trim)
        .filter(|l| l.len() > 10)
        .take(max_lines)
        .collect();

    let result = if lines.is_empty() {
        let truncated = &raw_text[..raw_text.len().min(MAX_CHARS)];
        match truncated.rfind(' ') {
            Some(pos) if pos > 0 => truncated[..pos].to_string(),
            _ => truncated.to_string(),
        }
    } else {
        lines.join("\n  ")
    };

    if result.len() > MAX_CHARS {
        result[..MAX_CHARS].to_string()
    } else {
        result
    }
}

/// Builds a compact role context block from ParsedJD structured fields.
/// Fills the {jd_text} slot in the fit score prompt with role shape, seniority,
/// culture signals, and nice-to-haves — NOT raw JD prose, NOT a copy of requirements.
fn build_jd_role_context(parsed_jd: &ParsedJD) -> String {
    let rs = &parsed_jd.role_signals;
    let role_shape = format!(
        "Seniority: {} | Startup: {} | IC-focused: {} | Research: {}",
        rs.seniority,
        if rs.is_startup { "yes" } else { "no" },
        if rs.is_ic_focused { "yes" } else { "no" },
        if rs.is_research { "yes" } else { "no" },
    );
    let soft = if parsed_jd.soft_signals.is_empty() {
        "None".to_string()
    } else {
        parsed_jd.soft_signals.join("; ")
    };
    format!(
        "{}\nTone: {:?}\nNice-to-haves: {}",
        role_shape, parsed_jd.detected_tone, soft
    )
}

/// Builds a full (untruncated) summary of the candidate's context entries for diagnostic use.
/// Dumps the complete raw_text per entry with no character cap — use only when verifying
/// that score variation is working. For production, use build_entries_summary() instead.
fn build_entries_summary_full(entries: &[ContextEntryRow]) -> String {
    entries
        .iter()
        .map(|e| {
            let company_or_name = e
                .data
                .get("company")
                .or_else(|| e.data.get("name"))
                .or_else(|| e.data.get("project_name"))
                .or_else(|| e.data.get("institution"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");

            let role = e
                .data
                .get("role")
                .or_else(|| e.data.get("degree"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let skills = e
                .tags
                .iter()
                .filter(|t| *t != &e.entry_type)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");

            let header = format!("[{}] {} — {}", e.entry_type, company_or_name, role);
            let meta = format!(
                "  Skills: {}\n  Contribution: {} | Impact: {:.2} | Recency: {:.2}",
                skills, e.contribution_type, e.impact_score, e.recency_score
            );

            match e.raw_text.as_deref().filter(|t| !t.trim().is_empty()) {
                Some(raw) => format!("{}\n{}\n  Context:\n    {}", header, meta, raw),
                None => format!("{}\n{}", header, meta),
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Builds a compact multi-field summary of the candidate's context entries for LLM prompts.
/// Each entry includes structural metadata (type, company, role, skills, contribution level,
/// impact/recency scores) plus up to 5 lines from raw_text — capped at 500 chars per entry
/// to keep token cost bounded. The raw_text snippet gives Claude actual evidence of what
/// the candidate did, which is critical for accurate fit scoring.
fn build_entries_summary(entries: &[ContextEntryRow]) -> String {
    entries
        .iter()
        .map(|e| {
            let company_or_name = e
                .data
                .get("company")
                .or_else(|| e.data.get("name"))
                .or_else(|| e.data.get("project_name"))
                .or_else(|| e.data.get("institution"))
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown");

            let role = e
                .data
                .get("role")
                .or_else(|| e.data.get("degree"))
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let skills = e
                .tags
                .iter()
                .filter(|t| *t != &e.entry_type)
                .take(5)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");

            let header = format!("[{}] {} — {}", e.entry_type, company_or_name, role);
            let meta = format!(
                "  Skills: {}\n  Contribution: {} | Impact: {:.2} | Recency: {:.2}",
                skills, e.contribution_type, e.impact_score, e.recency_score
            );

            match e.raw_text.as_deref().filter(|t| !t.trim().is_empty()) {
                Some(raw) => {
                    let snippet = extract_raw_text_snippet(raw, 5);
                    format!("{}\n{}\n  Context:\n    {}", header, meta, snippet)
                }
                None => format!("{}\n{}", header, meta),
            }
        })
        .collect::<Vec<_>>()
        .join("\n\n")
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
            selected_entry_ids: vec![],
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
            let tag_match = entry.tags.iter().any(|t| t.to_lowercase() == keyword_lower);

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
        selected_entry_ids: vec![],
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

    fn make_entry(entry_id: Uuid, tags: Vec<String>, raw_text: Option<String>) -> ContextEntryRow {
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
            quality_score: 1.0,
            quality_flags: vec![],
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
        let entries = vec![make_entry(Uuid::new_v4(), vec!["python".to_string()], None)];
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
        let entries = vec![make_entry(Uuid::new_v4(), vec!["rust".to_string()], None)];
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
        let entries = vec![make_entry(Uuid::new_v4(), vec!["rust".to_string()], None)];
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

    // ── extract_raw_text_snippet tests ────────────────────────────────────────

    #[test]
    fn extract_returns_up_to_n_lines() {
        let raw = (1..=10)
            .map(|i| format!("Line number {i} with enough content"))
            .collect::<Vec<_>>()
            .join("\n");
        let result = extract_raw_text_snippet(&raw, 3);
        assert!(result.lines().count() <= 3);
    }

    #[test]
    fn extract_skips_short_lines() {
        let raw = "---\n\nok\nThis is a real bullet point with substance\nAnother real line here";
        let result = extract_raw_text_snippet(raw, 5);
        assert!(!result.contains("---"));
        assert!(!result.contains("\nok\n"));
    }

    #[test]
    fn extract_fallback_truncates_at_500() {
        let raw = "a".repeat(1000);
        let result = extract_raw_text_snippet(&raw, 5);
        assert!(result.len() <= 500);
    }

    #[test]
    fn extract_caps_at_500_with_line_structure() {
        let line = "a".repeat(120);
        let raw = (0..6).map(|_| line.clone()).collect::<Vec<_>>().join("\n");
        let result = extract_raw_text_snippet(&raw, 6);
        assert!(result.len() <= 500);
    }

    #[test]
    fn extract_empty_returns_empty() {
        assert_eq!(extract_raw_text_snippet("", 5), "");
    }

    // ── build_jd_role_context tests ───────────────────────────────────────────

    #[test]
    fn role_context_contains_seniority() {
        let jd = make_parsed_jd(vec![]);
        let result = build_jd_role_context(&jd);
        assert!(result.contains(&jd.role_signals.seniority));
    }

    #[test]
    fn role_context_contains_tone() {
        let jd = make_parsed_jd(vec![]);
        let result = build_jd_role_context(&jd);
        assert!(result.contains("Tone:"));
    }

    #[test]
    fn role_context_contains_soft_signals() {
        use crate::generation::jd_parser::RoleSignals;
        let jd = ParsedJD {
            hard_requirements: vec![],
            soft_signals: vec!["Kubernetes".to_string(), "Kafka".to_string()],
            role_signals: RoleSignals {
                seniority: "senior".to_string(),
                is_startup: false,
                is_ic_focused: true,
                is_research: false,
            },
            keyword_inventory: vec![],
            detected_tone: crate::generation::jd_parser::JDTone::CollaborativeEnterprise,
        };
        let result = build_jd_role_context(&jd);
        assert!(result.contains("Kubernetes"));
        assert!(result.contains("Kafka"));
    }

    #[test]
    fn role_context_no_soft_signals_shows_none() {
        use crate::generation::jd_parser::RoleSignals;
        let jd = ParsedJD {
            hard_requirements: vec![],
            soft_signals: vec![],
            role_signals: RoleSignals {
                seniority: "mid".to_string(),
                is_startup: true,
                is_ic_focused: true,
                is_research: false,
            },
            keyword_inventory: vec![],
            detected_tone: crate::generation::jd_parser::JDTone::AggressiveStartup,
        };
        let result = build_jd_role_context(&jd);
        assert!(result.contains("None"));
    }

    #[test]
    fn role_context_does_not_duplicate_requirements() {
        use crate::generation::jd_parser::{Requirement, RoleSignals};
        let req_text = "Must have 10 years of Rust experience";
        let jd = ParsedJD {
            hard_requirements: vec![Requirement {
                text: req_text.to_string(),
                is_required: true,
            }],
            soft_signals: vec![],
            role_signals: RoleSignals {
                seniority: "senior".to_string(),
                is_startup: false,
                is_ic_focused: true,
                is_research: false,
            },
            keyword_inventory: vec![],
            detected_tone: crate::generation::jd_parser::JDTone::CollaborativeEnterprise,
        };
        let result = build_jd_role_context(&jd);
        assert!(!result.contains(req_text));
    }

    // ── build_entries_summary tests ───────────────────────────────────────────

    fn make_default_entry() -> ContextEntryRow {
        ContextEntryRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            version: 1,
            entry_type: "experience".to_string(),
            data: serde_json::json!({"company": "Acme Corp", "role": "Engineer"}),
            raw_text: None,
            recency_score: 0.8,
            impact_score: 0.8,
            tags: vec!["rust".to_string()],
            flagged_evergreen: false,
            contribution_type: "primary_contributor".to_string(),
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: Utc::now(),
        }
    }

    #[test]
    fn entries_summary_includes_raw_text_snippet() {
        let mut entry = make_default_entry();
        entry.raw_text = Some("Led a team of five engineers building distributed cache\nReduced latency by 40% through architecture changes".to_string());
        let result = build_entries_summary(&[entry]);
        assert!(result.contains("Led a team"));
    }

    #[test]
    fn entries_summary_includes_impact_score() {
        let mut entry = make_default_entry();
        entry.impact_score = 0.9;
        let result = build_entries_summary(&[entry]);
        assert!(result.contains("Impact:"));
        assert!(result.contains("0.90"));
    }

    #[test]
    fn entries_summary_includes_contribution_type() {
        let mut entry = make_default_entry();
        entry.contribution_type = "primary_contributor".to_string();
        let result = build_entries_summary(&[entry]);
        assert!(result.contains("primary_contributor"));
    }

    #[test]
    fn entries_summary_handles_none_raw_text() {
        let mut entry = make_default_entry();
        entry.raw_text = None;
        // Should not panic; should produce valid output without a Context block
        let result = build_entries_summary(&[entry]);
        assert!(!result.is_empty());
        assert!(!result.contains("Context:"));
    }
}
