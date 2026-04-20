//! Content Selector — ranks and selects context entries for resume generation.
//!
//! Reuses `context::scoring::compute_combined_score` for combined relevance scoring.
//! No LLM calls — reframe hints are populated by the generator via an optional LLM call.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::scoring::{compute_combined_score, ScoringWeights};
use crate::generation::generator::ResumeMode;
use crate::generation::jd_parser::{JDTone, ParsedJD};
use crate::models::context::ContextEntryRow;

// ────────────────────────────────────────────────────────────────────────────
// Data models
// ────────────────────────────────────────────────────────────────────────────

/// A context entry ranked by combined relevance score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RankedEntry {
    pub entry: ContextEntryRow,
    pub combined_score: f64,
    pub jd_relevance: f64,
}

/// Optional reframe suggestion for an entry, produced by a separate LLM call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReframeHint {
    pub entry_id: Uuid,
    pub suggested_framing: String,
}

/// Result of content selection — feeds directly into the generation prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionResult {
    pub selected_entries: Vec<RankedEntry>,
    pub excluded_entries: Vec<(Uuid, String)>, // (entry_id, reason)
    pub section_weights: HashMap<String, f32>,
    pub reframe_hints: Vec<ReframeHint>, // populated by generator if LLM available
}

// ────────────────────────────────────────────────────────────────────────────
// Selection algorithm
// ────────────────────────────────────────────────────────────────────────────

/// Section-level limits for single-page resume entry selection.
const EXPERIENCE_LIMIT: usize = 8;
const PROJECT_LIMIT: usize = 4;
const OTHER_LIMIT: usize = 3;

/// Section-level limits for multi-page CV entry selection (lifted significantly).
const CV_EXPERIENCE_LIMIT: usize = 20;
const CV_PROJECT_LIMIT: usize = 10;
const CV_OTHER_LIMIT: usize = 8;

/// Selects, ranks, and filters context entries for resume generation.
///
/// Algorithm:
/// 1. Compute `jd_relevance` per entry from keyword tag/text overlap
/// 2. Compute `combined_score` via existing context::scoring formula
/// 3. Sort descending by combined_score
/// 4. Apply per-section selection limits (higher for CV mode)
/// 5. Adjust section_weights based on JD tone signals
pub fn select_content(
    entries: Vec<ContextEntryRow>,
    parsed_jd: &ParsedJD,
    resume_mode: ResumeMode,
    persona: Option<&crate::models::resume::PersonaRow>,
) -> SelectionResult {
    let weights = ScoringWeights::default();

    // Score and rank all entries
    let mut ranked: Vec<RankedEntry> = entries
        .into_iter()
        .map(|entry| {
            let jd_relevance = compute_jd_relevance(&entry, parsed_jd);
            let base_score = compute_combined_score(
                entry.recency_score,
                entry.impact_score,
                jd_relevance,
                &weights,
            );
            let combined_score = (base_score * persona_multiplier(&entry, persona))
                .min(1.0)
                .max(0.0);
            RankedEntry {
                entry,
                combined_score,
                jd_relevance,
            }
        })
        .collect();

    // Sort descending — highest combined score first
    ranked.sort_by(|a, b| {
        b.combined_score
            .partial_cmp(&a.combined_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Apply section-aware selection limits (CV mode uses higher limits)
    let (selected_entries, excluded_entries) = apply_section_limits(ranked, resume_mode);

    // Adjust section weights per JD tone
    let section_weights = compute_section_weights(&parsed_jd.detected_tone);

    SelectionResult {
        selected_entries,
        excluded_entries,
        section_weights,
        reframe_hints: Vec::new(), // populated by generator via optional LLM call
    }
}

/// Computes JD relevance for a context entry based on keyword overlap.
///
/// Returns 0.0 if no keywords, otherwise: matched_weighted_score / total_weighted_score.
pub fn compute_jd_relevance(entry: &ContextEntryRow, parsed_jd: &ParsedJD) -> f64 {
    if parsed_jd.keyword_inventory.is_empty() {
        return 0.0;
    }

    let total_weight: f32 = parsed_jd
        .keyword_inventory
        .iter()
        .map(|k| k.weighted_score)
        .sum();

    if total_weight == 0.0 {
        return 0.0;
    }

    let matched_weight: f32 = parsed_jd
        .keyword_inventory
        .iter()
        .filter(|kw| {
            let kw_lower = kw.keyword.to_lowercase();
            let tag_hit = entry.tags.iter().any(|t| t.to_lowercase() == kw_lower);
            let text_hit = entry
                .raw_text
                .as_deref()
                .map(|t| t.to_lowercase().contains(&kw_lower))
                .unwrap_or(false);
            tag_hit || text_hit
        })
        .map(|kw| kw.weighted_score)
        .sum();

    (matched_weight / total_weight) as f64
}

/// Returns a score multiplier based on persona tag matching.
///
/// Suppressed beats emphasized: if an entry matches both, suppressed (0.2×) wins.
/// Returns 1.0 when no persona is active or no tags match.
fn persona_multiplier(
    entry: &ContextEntryRow,
    persona: Option<&crate::models::resume::PersonaRow>,
) -> f64 {
    let Some(p) = persona else { return 1.0 };
    let suppressed = entry
        .tags
        .iter()
        .any(|t| p.suppressed_tags.iter().any(|s| s.eq_ignore_ascii_case(t)));
    if suppressed {
        return 0.2;
    }
    let emphasized = entry
        .tags
        .iter()
        .any(|t| p.emphasized_tags.iter().any(|e| e.eq_ignore_ascii_case(t)));
    if emphasized {
        return 1.5;
    }
    1.0
}

/// Applies per-section limits and separates selected from excluded entries.
fn apply_section_limits(
    ranked: Vec<RankedEntry>,
    resume_mode: ResumeMode,
) -> (Vec<RankedEntry>, Vec<(Uuid, String)>) {
    let mut experience_count = 0usize;
    let mut project_count = 0usize;
    let mut other_count = 0usize;

    let (exp_limit, proj_limit, other_limit) = match resume_mode {
        ResumeMode::SinglePage => (EXPERIENCE_LIMIT, PROJECT_LIMIT, OTHER_LIMIT),
        ResumeMode::Cv => (CV_EXPERIENCE_LIMIT, CV_PROJECT_LIMIT, CV_OTHER_LIMIT),
    };

    let mut selected = Vec::new();
    let mut excluded = Vec::new();

    for ranked_entry in ranked {
        let section = ranked_entry.entry.entry_type.as_str();

        let (limit, count) = match section {
            "experience" => (exp_limit, &mut experience_count),
            "project" | "open_source" => (proj_limit, &mut project_count),
            _ => (other_limit, &mut other_count),
        };

        if *count < limit {
            *count += 1;
            selected.push(ranked_entry);
        } else {
            let reason = format!("Section limit reached ({} max for {})", limit, section);
            excluded.push((ranked_entry.entry.entry_id, reason));
        }
    }

    (selected, excluded)
}

/// Computes section weights adjusted by JD tone signals.
fn compute_section_weights(tone: &JDTone) -> HashMap<String, f32> {
    let mut weights: HashMap<String, f32> = HashMap::from([
        ("experience".to_string(), 0.60),
        ("project".to_string(), 0.20),
        ("education".to_string(), 0.10),
        ("skill".to_string(), 0.05),
        ("open_source".to_string(), 0.05),
    ]);

    match tone {
        JDTone::ResearchOriented => {
            // Boost publications and projects for research roles
            *weights.entry("publication".to_string()).or_insert(0.0) += 0.20;
            *weights.entry("project".to_string()).or_insert(0.0) += 0.10;
            *weights.entry("experience".to_string()).or_insert(0.0) -= 0.10;
        }
        JDTone::AggressiveStartup => {
            // Boost projects and open source for startup roles
            *weights.entry("project".to_string()).or_insert(0.0) += 0.15;
            *weights.entry("open_source".to_string()).or_insert(0.0) += 0.10;
            *weights.entry("experience".to_string()).or_insert(0.0) -= 0.05;
        }
        JDTone::CollaborativeEnterprise => {
            // Boost experience for enterprise roles
            *weights.entry("experience".to_string()).or_insert(0.0) += 0.10;
        }
        JDTone::ProductOriented => {
            // Slight boost to projects for product roles
            *weights.entry("project".to_string()).or_insert(0.0) += 0.10;
            *weights.entry("experience".to_string()).or_insert(0.0) += 0.05;
        }
    }

    weights
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::generator::ResumeMode;
    use crate::generation::jd_parser::{JDTone, KeywordEntry, ParsedJD, RoleSignals};
    use chrono::Utc;
    use serde_json::json;

    fn make_entry(
        entry_type: &str,
        tags: Vec<String>,
        recency: f64,
        impact: f64,
    ) -> ContextEntryRow {
        ContextEntryRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            version: 1,
            entry_type: entry_type.to_string(),
            data: json!({}),
            raw_text: None,
            recency_score: recency,
            impact_score: impact,
            tags,
            flagged_evergreen: false,
            contribution_type: "primary_contributor".to_string(),
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: Utc::now(),
        }
    }

    fn make_parsed_jd(keywords: &[&str], tone: JDTone) -> ParsedJD {
        ParsedJD {
            hard_requirements: vec![],
            soft_signals: vec![],
            role_signals: RoleSignals {
                is_startup: false,
                is_ic_focused: true,
                is_research: false,
                seniority: "senior".to_string(),
            },
            keyword_inventory: keywords
                .iter()
                .map(|kw| KeywordEntry {
                    keyword: kw.to_string(),
                    frequency: 3,
                    position_weight: 0.8,
                    weighted_score: 2.4,
                })
                .collect(),
            detected_tone: tone,
        }
    }

    #[test]
    fn test_high_score_entry_selected_first() {
        let entries = vec![
            make_entry("experience", vec!["rust".to_string()], 1.0, 1.0),
            make_entry("experience", vec![], 0.1, 0.1),
        ];
        let parsed_jd = make_parsed_jd(&["rust"], JDTone::AggressiveStartup);
        let result = select_content(entries, &parsed_jd, ResumeMode::SinglePage, None);

        assert!(
            result.selected_entries[0].combined_score > result.selected_entries[1].combined_score,
            "Higher-scoring entry must be first"
        );
    }

    #[test]
    fn test_experience_section_capped_at_8() {
        let entries: Vec<_> = (0..12)
            .map(|_| make_entry("experience", vec![], 0.5, 0.5))
            .collect();
        let parsed_jd = make_parsed_jd(&[], JDTone::CollaborativeEnterprise);
        let result = select_content(entries, &parsed_jd, ResumeMode::SinglePage, None);

        let selected_exp = result
            .selected_entries
            .iter()
            .filter(|e| e.entry.entry_type == "experience")
            .count();
        assert_eq!(selected_exp, 8, "Experience capped at 8");

        let excluded_exp = result
            .excluded_entries
            .iter()
            .filter(|(_, reason)| reason.contains("experience"))
            .count();
        assert_eq!(excluded_exp, 4, "4 experience entries excluded");
    }

    #[test]
    fn test_project_section_capped_at_4() {
        let entries: Vec<_> = (0..7)
            .map(|_| make_entry("project", vec![], 0.5, 0.5))
            .collect();
        let parsed_jd = make_parsed_jd(&[], JDTone::CollaborativeEnterprise);
        let result = select_content(entries, &parsed_jd, ResumeMode::SinglePage, None);

        let selected = result
            .selected_entries
            .iter()
            .filter(|e| e.entry.entry_type == "project")
            .count();
        assert_eq!(selected, 4, "Project capped at 4");
    }

    #[test]
    fn test_open_source_counts_toward_project_limit() {
        // open_source shares the project limit bucket
        let entries: Vec<_> = (0..6)
            .map(|_| make_entry("open_source", vec![], 0.5, 0.5))
            .collect();
        let parsed_jd = make_parsed_jd(&[], JDTone::AggressiveStartup);
        let result = select_content(entries, &parsed_jd, ResumeMode::SinglePage, None);

        let selected = result
            .selected_entries
            .iter()
            .filter(|e| e.entry.entry_type == "open_source")
            .count();
        assert_eq!(
            selected, 4,
            "open_source capped at 4 (shared project limit)"
        );
    }

    #[test]
    fn test_research_tone_adds_publication_weight() {
        let weights = compute_section_weights(&JDTone::ResearchOriented);
        assert!(
            *weights.get("publication").unwrap_or(&0.0) > 0.0,
            "Research tone must boost publication weight"
        );
    }

    #[test]
    fn test_startup_tone_boosts_open_source_weight() {
        let base = compute_section_weights(&JDTone::CollaborativeEnterprise);
        let startup = compute_section_weights(&JDTone::AggressiveStartup);
        assert!(
            startup.get("open_source").unwrap_or(&0.0) > base.get("open_source").unwrap_or(&0.0),
            "Startup tone must have higher open_source weight"
        );
    }

    #[test]
    fn test_jd_relevance_zero_when_no_keywords() {
        let entry = make_entry("experience", vec!["rust".to_string()], 1.0, 1.0);
        let parsed_jd = make_parsed_jd(&[], JDTone::CollaborativeEnterprise);
        assert_eq!(compute_jd_relevance(&entry, &parsed_jd), 0.0);
    }

    #[test]
    fn test_jd_relevance_perfect_tag_match() {
        let entry = make_entry("experience", vec!["rust".to_string()], 1.0, 1.0);
        let parsed_jd = make_parsed_jd(&["rust"], JDTone::AggressiveStartup);
        let rel = compute_jd_relevance(&entry, &parsed_jd);
        assert!(
            (rel - 1.0).abs() < f64::EPSILON,
            "Single-keyword tag match should be 1.0, got {rel}"
        );
    }

    #[test]
    fn test_jd_relevance_partial_match() {
        let entry = make_entry(
            "experience",
            vec!["rust".to_string()], // matches "rust" only
            1.0,
            1.0,
        );
        // Two keywords: rust + kubernetes — only rust matches
        let parsed_jd = make_parsed_jd(&["rust", "kubernetes"], JDTone::AggressiveStartup);
        let rel = compute_jd_relevance(&entry, &parsed_jd);
        assert!(
            rel > 0.0 && rel < 1.0,
            "Partial match should be 0 < rel < 1, got {rel}"
        );
    }

    #[test]
    fn test_reframe_hints_empty_by_default() {
        let result = select_content(
            vec![],
            &make_parsed_jd(&[], JDTone::ProductOriented),
            ResumeMode::SinglePage,
            None,
        );
        assert!(result.reframe_hints.is_empty());
    }

    fn make_persona(emphasized: &[&str], suppressed: &[&str]) -> crate::models::resume::PersonaRow {
        crate::models::resume::PersonaRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            name: "Test Persona".to_string(),
            emphasized_tags: emphasized.iter().map(|s| s.to_string()).collect(),
            suppressed_tags: suppressed.iter().map(|s| s.to_string()).collect(),
            tone_preference: None,
            section_order: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn test_persona_emphasize_boosts_score() {
        let entry = make_entry("experience", vec!["ml".to_string()], 0.5, 0.5);
        let persona = make_persona(&["ml"], &[]);
        let multiplier = persona_multiplier(&entry, Some(&persona));
        assert_eq!(multiplier, 1.5, "Emphasized tag must boost by 1.5×");
    }

    #[test]
    fn test_persona_suppress_reduces_score() {
        let entry = make_entry("experience", vec!["react".to_string()], 0.5, 0.5);
        let persona = make_persona(&[], &["react"]);
        let multiplier = persona_multiplier(&entry, Some(&persona));
        assert_eq!(multiplier, 0.2, "Suppressed tag must reduce to 0.2×");
    }

    #[test]
    fn test_persona_suppress_beats_emphasize() {
        // Entry has both an emphasized and a suppressed tag — suppressed wins.
        let entry = make_entry(
            "experience",
            vec!["ml".to_string(), "react".to_string()],
            0.5,
            0.5,
        );
        let persona = make_persona(&["ml"], &["react"]);
        let multiplier = persona_multiplier(&entry, Some(&persona));
        assert_eq!(multiplier, 0.2, "Suppressed must win over emphasized");
    }

    #[test]
    fn test_persona_none_is_noop() {
        let entry = make_entry("experience", vec!["ml".to_string()], 0.6, 0.6);
        let multiplier = persona_multiplier(&entry, None);
        assert_eq!(multiplier, 1.0, "No persona must return 1.0 multiplier");
    }
}
