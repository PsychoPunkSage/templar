#![allow(dead_code)]

//! Scope inflation check — pure string matching, no LLM required.
//!
//! Runs before LLM scoring to save tokens on obvious violations.
//! Checks only the first 3 words of the bullet (the action verb position).
//!
//! Contribution type rules (mirroring `generation/tone.rs` SCOPE_INSTRUCTION):
//! - `sole_author` / `primary_contributor` — no restrictions
//! - `team_member` — may not use sole-ownership verbs (Architected, Built, etc.)
//! - `reviewer` — may not use implementation/ownership verbs
//! - unknown types — treated conservatively as `team_member`

/// Forbidden action verbs for `team_member` entries.
/// These imply sole ownership or leadership that a team member cannot claim.
const TEAM_MEMBER_FORBIDDEN: &[&str] = &[
    "Architected",
    "Designed",
    "Built",
    "Created",
    "Owned",
    "Led",
    "Spearheaded",
    "Pioneered",
];

/// Forbidden action verbs for `reviewer` entries.
/// Reviewers can evaluate/assess; they cannot claim to have implemented or built.
const REVIEWER_FORBIDDEN: &[&str] = &[
    "Architected",
    "Designed",
    "Built",
    "Created",
    "Owned",
    "Led",
    "Spearheaded",
    "Implemented",
    "Developed",
];

/// Checks whether a bullet's action verb is appropriate for the given contribution type.
///
/// Returns `Some(reason)` if a forbidden verb is detected — caller should reject or rewrite.
/// Returns `None` if the bullet passes scope validation.
///
/// The check is:
/// 1. Case-insensitive
/// 2. Limited to the first 3 words of the bullet (where action verbs appear)
/// 3. Unknown contribution types are treated as `team_member` (conservative)
pub fn check_scope_inflation(bullet_text: &str, contribution_type: &str) -> Option<String> {
    let forbidden = match contribution_type {
        "sole_author" | "primary_contributor" => return None,
        "team_member" => TEAM_MEMBER_FORBIDDEN,
        "reviewer" => REVIEWER_FORBIDDEN,
        // Unknown contribution type: treat conservatively as team_member
        _ => TEAM_MEMBER_FORBIDDEN,
    };

    // Extract first 3 words (where action verb lives)
    let first_words: Vec<&str> = bullet_text.split_whitespace().take(3).collect();

    for word in &first_words {
        // Strip trailing punctuation for matching
        let clean = word.trim_end_matches(|c: char| !c.is_alphanumeric());
        for &forbidden_verb in forbidden {
            if clean.eq_ignore_ascii_case(forbidden_verb) {
                return Some(format!(
                    "Scope inflation: '{}' is a sole-ownership verb and cannot be used for a '{}' contribution entry. \
                    Use collaborative language instead (e.g., 'Contributed to', 'Implemented as part of team').",
                    clean, contribution_type
                ));
            }
        }
    }

    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_team_member_architected_is_inflation() {
        let result = check_scope_inflation(
            "Architected the distributed caching layer reducing latency by 40%",
            "team_member",
        );
        assert!(
            result.is_some(),
            "Architected must be flagged for team_member"
        );
        let reason = result.unwrap();
        assert!(
            reason.contains("Architected"),
            "reason should mention the verb"
        );
        assert!(
            reason.contains("team_member"),
            "reason should mention contribution type"
        );
    }

    #[test]
    fn test_team_member_contributed_is_ok() {
        let result = check_scope_inflation(
            "Contributed to the distributed caching layer reducing latency by 40%",
            "team_member",
        );
        assert!(
            result.is_none(),
            "Contributed to must be allowed for team_member"
        );
    }

    #[test]
    fn test_team_member_owned_is_inflation() {
        let result = check_scope_inflation(
            "Owned the on-call rotation and resolved 95% of incidents within SLA",
            "team_member",
        );
        assert!(result.is_some(), "Owned must be flagged for team_member");
    }

    #[test]
    fn test_reviewer_implemented_is_inflation() {
        let result = check_scope_inflation(
            "Implemented the authentication service for 2M users",
            "reviewer",
        );
        assert!(result.is_some(), "Implemented must be flagged for reviewer");
    }

    #[test]
    fn test_reviewer_reviewed_is_ok() {
        let result = check_scope_inflation(
            "Reviewed 50+ pull requests for security vulnerabilities in the auth service",
            "reviewer",
        );
        assert!(result.is_none(), "Reviewed must be allowed for reviewer");
    }

    #[test]
    fn test_sole_author_architected_is_ok() {
        let result = check_scope_inflation(
            "Architected the distributed caching layer reducing p99 latency by 40%",
            "sole_author",
        );
        assert!(
            result.is_none(),
            "Architected must be allowed for sole_author"
        );
    }

    #[test]
    fn test_primary_contributor_built_is_ok() {
        let result = check_scope_inflation(
            "Built the CI/CD pipeline cutting deployment time from 45 minutes to 8 minutes",
            "primary_contributor",
        );
        assert!(
            result.is_none(),
            "Built must be allowed for primary_contributor"
        );
    }

    #[test]
    fn test_case_insensitive_detection() {
        // lowercase "architected" should still be caught
        let result =
            check_scope_inflation("architected the distributed caching layer", "team_member");
        assert!(
            result.is_some(),
            "lowercase 'architected' must be caught for team_member"
        );
    }

    #[test]
    fn test_clean_bullet_no_inflation() {
        let result = check_scope_inflation(
            "Collaborated on migrating 3 legacy services to gRPC, reducing inter-service latency by 30%",
            "team_member",
        );
        assert!(
            result.is_none(),
            "Collaborated on must be allowed for team_member"
        );
    }

    #[test]
    fn test_unknown_contribution_type_conservative() {
        // Unknown types should be treated as team_member (conservative)
        let result = check_scope_inflation(
            "Architected the entire data platform from scratch",
            "intern",
        );
        assert!(
            result.is_some(),
            "Unknown contribution type should be treated conservatively (as team_member)"
        );
    }
}
