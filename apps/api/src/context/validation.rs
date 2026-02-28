#![allow(dead_code)]

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactGap {
    pub bullet: String,
    pub reason: String,
    pub suggestion: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactValidationResult {
    pub passed: bool,
    pub missing: Vec<ImpactGap>,
    pub suggestions: Vec<String>,
}

const VAGUE_VERBS: &[&str] = &[
    "improved",
    "enhanced",
    "helped",
    "worked on",
    "assisted",
    "supported",
    "participated",
    "involved",
];

const VAGUE_SCALE_WORDS: &[&str] = &[
    "significant",
    "major",
    "large",
    "huge",
    "massive",
    "substantial",
    "considerable",
    "great",
    "many",
    "numerous",
    "various",
    "several",
];

/// Validates a single bullet for impact quantification.
///
/// PASS conditions:
/// - Contains a digit (number)
/// - Contains `[LOW_METRICS]` marker
/// - Contains `~N` estimate patterns
/// - Contains `%`, `$`, `€`, `£`
/// - Contains `Nx` multiplier patterns
///
/// FAIL conditions:
/// - Vague verbs without metrics
/// - Vague scale words without numbers
pub fn validate_impact(text: &str) -> ImpactValidationResult {
    let text_lower = text.to_lowercase();

    let has_digit = text.chars().any(|c| c.is_ascii_digit());
    let has_low_metrics = text.contains("[LOW_METRICS]");
    let has_tilde = text.contains('~') && text.chars().any(|c| c.is_ascii_digit());
    let has_percent = text.contains('%');
    let has_currency = text.contains('$') || text.contains('€') || text.contains('£');
    let has_multiplier = has_digit
        && (text_lower.contains("x faster")
            || text_lower.contains("x improvement")
            || text_lower.contains("x reduction")
            || text_lower.contains("x more"));

    let is_quantified =
        has_digit || has_low_metrics || has_tilde || has_percent || has_currency || has_multiplier;

    if is_quantified {
        return ImpactValidationResult {
            passed: true,
            missing: vec![],
            suggestions: vec![],
        };
    }

    let mut missing = Vec::new();
    let mut suggestions = Vec::new();

    for &vague in VAGUE_VERBS {
        if text_lower.contains(vague) {
            missing.push(ImpactGap {
                bullet: text.to_string(),
                reason: format!("Contains vague verb '{}' without quantified impact", vague),
                suggestion: format!(
                    "Add a metric: e.g., '{}' by X%, resulting in Y reduction, or tag with [LOW_METRICS]",
                    vague
                ),
            });
            suggestions.push(format!(
                "Quantify '{}': How much? Add a number, percentage, or time saved.",
                vague
            ));
            break;
        }
    }

    for &vague_scale in VAGUE_SCALE_WORDS {
        if text_lower.contains(vague_scale) {
            missing.push(ImpactGap {
                bullet: text.to_string(),
                reason: format!("Uses vague scale word '{}' without a number", vague_scale),
                suggestion: format!(
                    "Replace '{}' with a specific number: e.g., '5x', '40%', '3 weeks'",
                    vague_scale
                ),
            });
            suggestions.push(format!(
                "Replace '{}' with a specific number or percentage.",
                vague_scale
            ));
            break;
        }
    }

    if missing.is_empty() {
        missing.push(ImpactGap {
            bullet: text.to_string(),
            reason: "No quantified outcome found".to_string(),
            suggestion: "Add a metric (number, %, time, or use [LOW_METRICS] if unavailable)"
                .to_string(),
        });
        suggestions.push(
            "Add a specific number, percentage, or time metric. If data unavailable, append [LOW_METRICS].".to_string(),
        );
    }

    ImpactValidationResult {
        passed: false,
        missing,
        suggestions,
    }
}

/// Validates a batch of bullets, collecting all failures.
pub fn validate_bullets(bullets: &[String]) -> ImpactValidationResult {
    let mut all_missing = Vec::new();
    let mut all_suggestions = Vec::new();
    let mut any_failed = false;

    for bullet in bullets {
        let result = validate_impact(bullet);
        if !result.passed {
            any_failed = true;
            all_missing.extend(result.missing);
            all_suggestions.extend(result.suggestions);
        }
    }

    ImpactValidationResult {
        passed: !any_failed,
        missing: all_missing,
        suggestions: all_suggestions,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pass_with_percentage() {
        assert!(validate_impact("Reduced latency by 40% through caching").passed);
    }

    #[test]
    fn test_pass_with_dollar_amount() {
        assert!(validate_impact("Saved $50,000 annually by optimizing queries").passed);
    }

    #[test]
    fn test_pass_with_count() {
        assert!(validate_impact("Built 3 microservices handling 10k rps").passed);
    }

    #[test]
    fn test_pass_with_low_metrics_marker() {
        assert!(validate_impact("Improved system performance [LOW_METRICS]").passed);
    }

    #[test]
    fn test_pass_with_tilde_estimate() {
        assert!(validate_impact("Reduced deployment time by ~2 hours").passed);
    }

    #[test]
    fn test_pass_with_euro() {
        assert!(validate_impact("Generated €200k in new revenue").passed);
    }

    #[test]
    fn test_pass_with_digit_in_tech() {
        assert!(validate_impact("Designed REST API serving 1M requests/day").passed);
    }

    #[test]
    fn test_pass_with_k_notation() {
        assert!(validate_impact("Processed 100k+ records daily").passed);
    }

    #[test]
    fn test_pass_time_saved() {
        assert!(validate_impact("Reduced build time from 45 minutes to 8 minutes").passed);
    }

    #[test]
    fn test_pass_team_count() {
        assert!(validate_impact("Trained 15 engineers on new deployment process").passed);
    }

    #[test]
    fn test_fail_improved_without_metrics() {
        let r = validate_impact("Improved the user experience");
        assert!(!r.passed);
        assert!(!r.missing.is_empty());
        assert!(r.missing[0].reason.contains("vague verb"));
    }

    #[test]
    fn test_fail_enhanced_without_metrics() {
        assert!(!validate_impact("Enhanced the database performance").passed);
    }

    #[test]
    fn test_fail_helped_without_metrics() {
        assert!(!validate_impact("Helped the team deliver projects").passed);
    }

    #[test]
    fn test_fail_worked_on() {
        assert!(!validate_impact("Worked on backend infrastructure").passed);
    }

    #[test]
    fn test_fail_significant_without_number() {
        let r = validate_impact("Achieved significant performance improvements");
        assert!(!r.passed);
        assert!(r.missing[0].reason.contains("vague scale word"));
    }

    #[test]
    fn test_fail_major_no_number() {
        assert!(!validate_impact("Led major improvements to the codebase").passed);
    }

    #[test]
    fn test_fail_various_projects() {
        assert!(!validate_impact("Led various projects across teams").passed);
    }

    #[test]
    fn test_fail_numerous() {
        assert!(!validate_impact("Managed numerous client accounts").passed);
    }

    #[test]
    fn test_fail_no_metrics_at_all() {
        let r = validate_impact("Architected the authentication system");
        assert!(!r.passed);
        assert!(!r.suggestions.is_empty());
    }

    #[test]
    fn test_fail_collaborated_no_metrics() {
        assert!(!validate_impact("Collaborated on the platform migration").passed);
    }

    #[test]
    fn test_fail_assisted_no_metrics() {
        assert!(!validate_impact("Assisted with deployment automation").passed);
    }

    #[test]
    fn test_validate_bullets_mixed() {
        let bullets = vec![
            "Reduced latency by 40%".to_string(),
            "Improved the user experience".to_string(),
        ];
        let r = validate_bullets(&bullets);
        assert!(!r.passed);
        assert_eq!(r.missing.len(), 1);
    }

    #[test]
    fn test_validate_bullets_all_pass() {
        let bullets = vec![
            "Reduced latency by 40%".to_string(),
            "Processed 100k records [LOW_METRICS]".to_string(),
        ];
        assert!(validate_bullets(&bullets).passed);
    }

    #[test]
    fn test_validate_bullets_empty() {
        assert!(validate_bullets(&[]).passed);
    }
}
