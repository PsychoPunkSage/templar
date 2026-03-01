#![allow(dead_code)]

//! Tone calibration — maps detected JD tone to verb sets, filters by contribution type.
//!
//! CRITICAL: Tone intersects with the SCOPE_INSTRUCTION constraint.
//! A `team_member` entry NEVER gets AggressiveStartup sole-owner verbs,
//! regardless of detected JD tone. This is a hard architectural rule.

use crate::generation::jd_parser::JDTone;

/// Verb sets and phrasing calibrated to a specific JD tone.
#[derive(Debug, Clone)]
pub struct ToneExamples {
    pub strong_verbs: Vec<&'static str>,
    pub ownership_prefix: &'static str,
    pub avoid_verbs: Vec<&'static str>,
}

/// Returns tone-calibrated verb sets for the detected JD tone.
pub fn get_tone_examples(tone: &JDTone) -> ToneExamples {
    match tone {
        JDTone::AggressiveStartup => ToneExamples {
            strong_verbs: vec![
                "Architected",
                "Spearheaded",
                "Owned",
                "Drove",
                "Built",
                "Shipped",
                "Launched",
                "Led",
            ],
            ownership_prefix: "end-to-end ownership of",
            avoid_verbs: vec!["assisted", "helped", "supported", "participated in"],
        },
        JDTone::CollaborativeEnterprise => ToneExamples {
            strong_verbs: vec![
                "Contributed to",
                "Partnered with",
                "Supported",
                "Enabled",
                "Collaborated on",
                "Facilitated",
            ],
            ownership_prefix: "as part of a team,",
            avoid_verbs: vec!["architected", "spearheaded", "solely built", "owned end-to-end"],
        },
        JDTone::ResearchOriented => ToneExamples {
            strong_verbs: vec![
                "Investigated",
                "Designed and evaluated",
                "Published",
                "Proposed",
                "Analyzed",
                "Studied",
            ],
            ownership_prefix: "research into",
            avoid_verbs: vec!["shipped", "launched", "moved fast", "disrupted"],
        },
        JDTone::ProductOriented => ToneExamples {
            strong_verbs: vec![
                "Shipped",
                "Delivered",
                "Launched",
                "Improved",
                "Reduced friction for",
                "Enabled",
            ],
            ownership_prefix: "shipped",
            avoid_verbs: vec!["investigated", "evaluated", "researched", "proposed"],
        },
    }
}

/// Verbs that signal sole-author ownership — never allowed for team_member entries.
const SOLE_OWNER_VERBS: &[&str] = &[
    "Architected",
    "Spearheaded",
    "Owned",
    "Drove",
    "Led",
    "Built",
    "Designed",
];

/// Verbs appropriate for reviewer contribution type.
const REVIEWER_VERBS: &[&str] = &["Reviewed", "Evaluated", "Assessed", "Audited", "Analyzed"];

/// Filters a verb set based on the entry's contribution type.
///
/// CRITICAL: `team_member` entries cannot use sole-owner verbs even if the JD is AggressiveStartup.
/// `reviewer` entries are restricted to reviewer-appropriate verbs regardless of tone.
pub fn filter_verbs_for_contribution<'a>(
    verbs: &[&'a str],
    contribution_type: &str,
) -> Vec<&'a str> {
    match contribution_type {
        "sole_author" | "primary_contributor" => verbs.to_vec(),
        "team_member" => verbs
            .iter()
            .filter(|&&v| {
                !SOLE_OWNER_VERBS
                    .iter()
                    .any(|&sv| sv.eq_ignore_ascii_case(v))
            })
            .copied()
            .collect(),
        "reviewer" => REVIEWER_VERBS.to_vec(),
        // Unknown contribution type — be conservative, treat as team_member
        _ => verbs
            .iter()
            .filter(|&&v| {
                !SOLE_OWNER_VERBS
                    .iter()
                    .any(|&sv| sv.eq_ignore_ascii_case(v))
            })
            .copied()
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_startup_tone_includes_architected() {
        let t = get_tone_examples(&JDTone::AggressiveStartup);
        assert!(t.strong_verbs.contains(&"Architected"));
        assert!(t.strong_verbs.contains(&"Spearheaded"));
    }

    #[test]
    fn test_enterprise_tone_avoids_sole_language() {
        let t = get_tone_examples(&JDTone::CollaborativeEnterprise);
        assert!(t.avoid_verbs.contains(&"architected"));
        assert!(t.avoid_verbs.contains(&"spearheaded"));
    }

    #[test]
    fn test_research_tone_includes_published() {
        let t = get_tone_examples(&JDTone::ResearchOriented);
        assert!(t.strong_verbs.contains(&"Published"));
        assert!(t.strong_verbs.contains(&"Investigated"));
    }

    #[test]
    fn test_product_tone_includes_shipped() {
        let t = get_tone_examples(&JDTone::ProductOriented);
        assert!(t.strong_verbs.contains(&"Shipped"));
        assert!(t.strong_verbs.contains(&"Launched"));
    }

    #[test]
    fn test_team_member_filters_sole_owner_verbs() {
        let verbs = vec!["Architected", "Contributed to", "Owned", "Collaborated on"];
        let filtered = filter_verbs_for_contribution(&verbs, "team_member");
        assert!(
            !filtered.contains(&"Architected"),
            "team_member must not get Architected"
        );
        assert!(
            !filtered.contains(&"Owned"),
            "team_member must not get Owned"
        );
        assert!(
            filtered.contains(&"Contributed to"),
            "team_member should keep collaborative verbs"
        );
        assert!(filtered.contains(&"Collaborated on"));
    }

    #[test]
    fn test_sole_author_keeps_all_verbs() {
        let verbs = vec!["Architected", "Contributed to", "Owned"];
        let filtered = filter_verbs_for_contribution(&verbs, "sole_author");
        assert_eq!(filtered.len(), verbs.len(), "sole_author keeps all verbs");
    }

    #[test]
    fn test_primary_contributor_keeps_all_verbs() {
        let verbs = vec!["Architected", "Led", "Built"];
        let filtered = filter_verbs_for_contribution(&verbs, "primary_contributor");
        assert_eq!(filtered.len(), verbs.len());
    }

    #[test]
    fn test_reviewer_gets_review_verbs_only() {
        let verbs = vec!["Architected", "Contributed to"];
        let filtered = filter_verbs_for_contribution(&verbs, "reviewer");
        assert!(
            filtered.contains(&"Reviewed"),
            "reviewer must get Reviewed"
        );
        assert!(
            filtered.contains(&"Evaluated"),
            "reviewer must get Evaluated"
        );
        // Original verbs replaced by reviewer set
        assert!(!filtered.contains(&"Architected"));
    }

    #[test]
    fn test_unknown_contribution_type_treated_conservatively() {
        let verbs = vec!["Architected", "Contributed to"];
        let filtered = filter_verbs_for_contribution(&verbs, "unknown_type");
        // Conservative: filters sole-owner verbs
        assert!(!filtered.contains(&"Architected"));
        assert!(filtered.contains(&"Contributed to"));
    }

    /// CRITICAL INTEGRATION: team_member + AggressiveStartup tone must still
    /// exclude sole-owner verbs. This is the core scope inflation guard.
    #[test]
    fn test_startup_tone_team_member_never_gets_sole_owner_verbs() {
        let startup_tone = get_tone_examples(&JDTone::AggressiveStartup);
        let filtered =
            filter_verbs_for_contribution(&startup_tone.strong_verbs, "team_member");
        assert!(
            !filtered.contains(&"Architected"),
            "CRITICAL: team_member must never get Architected even in startup tone"
        );
        assert!(
            !filtered.contains(&"Owned"),
            "CRITICAL: team_member must never get Owned even in startup tone"
        );
        assert!(
            !filtered.contains(&"Spearheaded"),
            "CRITICAL: team_member must never get Spearheaded even in startup tone"
        );
    }
}
