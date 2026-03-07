#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// Non-blocking quality assessment for a context entry bullet.
///
/// Replaces the old pass/fail `ImpactValidationResult`. Ingest always proceeds;
/// quality metadata is stored on the entry and surfaced as UI hints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactQuality {
    /// 0.0–1.0: 1.0 = fully quantified, 0.0 = entirely vague.
    pub quality_score: f32,
    /// Machine-readable flags, e.g. ["missing_metric", "vague_verb:improved"]
    pub flags: Vec<String>,
    /// Human-readable improvement suggestions shown to the user.
    pub suggestions: Vec<String>,
}

impl ImpactQuality {
    /// Returns true if quality is considered acceptable (score ≥ 0.5).
    pub fn is_acceptable(&self) -> bool {
        self.quality_score >= 0.5
    }

    /// Merge quality from multiple bullets into one aggregate.
    pub fn aggregate(qualities: &[ImpactQuality]) -> Self {
        if qualities.is_empty() {
            return ImpactQuality {
                quality_score: 1.0,
                flags: vec![],
                suggestions: vec![],
            };
        }
        let avg = qualities.iter().map(|q| q.quality_score).sum::<f32>() / qualities.len() as f32;
        let flags: Vec<String> = qualities
            .iter()
            .flat_map(|q| q.flags.iter().cloned())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        let suggestions: Vec<String> = qualities
            .iter()
            .flat_map(|q| q.suggestions.iter().cloned())
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        ImpactQuality {
            quality_score: avg,
            flags,
            suggestions,
        }
    }
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

/// Assesses the impact quality of a single bullet string.
///
/// Always returns an `ImpactQuality` — never blocks ingest.
/// A score of 1.0 means fully quantified; 0.3 means no metrics at all.
///
/// HIGH quality (score 1.0): contains digit, %, $, [LOW_METRICS], ~N, or Nx multiplier
/// MEDIUM quality (score 0.5): no metrics but no vague language
/// LOW quality (score 0.3–0.4): vague verbs or vague scale words
pub fn validate_impact(text: &str) -> ImpactQuality {
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
        return ImpactQuality {
            quality_score: 1.0,
            flags: vec![],
            suggestions: vec![],
        };
    }

    let mut flags = Vec::new();
    let mut suggestions = Vec::new();
    let mut quality_score: f32 = 0.5; // default medium quality for no-metric bullets

    // Check for vague verbs
    for &vague in VAGUE_VERBS {
        if text_lower.contains(vague) {
            flags.push(format!("vague_verb:{}", vague.replace(' ', "_")));
            suggestions.push(format!(
                "Quantify '{}': Add a number, percentage, or time metric. If data unavailable, append [LOW_METRICS].",
                vague
            ));
            quality_score = 0.4;
            break;
        }
    }

    // Check for vague scale words
    for &vague_scale in VAGUE_SCALE_WORDS {
        if text_lower.contains(vague_scale) {
            flags.push(format!("vague_scale:{}", vague_scale));
            suggestions.push(format!(
                "Replace '{}' with a specific number or percentage (e.g. '5x', '40%', '3 weeks').",
                vague_scale
            ));
            quality_score = quality_score.min(0.4);
            break;
        }
    }

    // No metrics at all in an otherwise clean bullet
    if flags.is_empty() {
        flags.push("missing_metric".to_string());
        suggestions.push(
            "Add a specific number, percentage, or time metric. If data unavailable, append [LOW_METRICS].".to_string(),
        );
        quality_score = 0.5; // medium — no vague language, just no numbers
    }

    ImpactQuality {
        quality_score,
        flags,
        suggestions,
    }
}

/// Assesses quality across a batch of bullets, returning an aggregate.
pub fn validate_bullets(bullets: &[String]) -> ImpactQuality {
    let qualities: Vec<_> = bullets.iter().map(|b| validate_impact(b)).collect();
    ImpactQuality::aggregate(&qualities)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_high_quality_with_percentage() {
        let q = validate_impact("Reduced latency by 40% through caching");
        assert_eq!(q.quality_score, 1.0);
        assert!(q.flags.is_empty());
    }

    #[test]
    fn test_high_quality_with_dollar_amount() {
        let q = validate_impact("Saved $50,000 annually by optimizing queries");
        assert_eq!(q.quality_score, 1.0);
    }

    #[test]
    fn test_high_quality_with_count() {
        let q = validate_impact("Built 3 microservices handling 10k rps");
        assert_eq!(q.quality_score, 1.0);
    }

    #[test]
    fn test_high_quality_with_low_metrics_marker() {
        let q = validate_impact("Improved system performance [LOW_METRICS]");
        assert_eq!(q.quality_score, 1.0);
    }

    #[test]
    fn test_high_quality_with_tilde_estimate() {
        let q = validate_impact("Reduced deployment time by ~2 hours");
        assert_eq!(q.quality_score, 1.0);
    }

    #[test]
    fn test_high_quality_with_euro() {
        let q = validate_impact("Generated €200k in new revenue");
        assert_eq!(q.quality_score, 1.0);
    }

    #[test]
    fn test_high_quality_with_digit() {
        let q = validate_impact("Designed REST API serving 1M requests/day");
        assert_eq!(q.quality_score, 1.0);
    }

    #[test]
    fn test_high_quality_team_count() {
        let q = validate_impact("Trained 15 engineers on new deployment process");
        assert_eq!(q.quality_score, 1.0);
    }

    // --- Low quality but NOT blocked ---

    #[test]
    fn test_low_quality_vague_verb_improved() {
        let q = validate_impact("Improved the user experience");
        assert!(q.quality_score < 0.5);
        assert!(q.flags.iter().any(|f| f.starts_with("vague_verb:")));
        assert!(!q.suggestions.is_empty());
    }

    #[test]
    fn test_low_quality_vague_verb_enhanced() {
        let q = validate_impact("Enhanced the database performance");
        assert!(q.quality_score < 0.5);
        assert!(q.flags.iter().any(|f| f.starts_with("vague_verb:")));
    }

    #[test]
    fn test_low_quality_helped() {
        let q = validate_impact("Helped the team deliver projects");
        assert!(q.quality_score < 0.5);
    }

    #[test]
    fn test_low_quality_worked_on() {
        let q = validate_impact("Worked on backend infrastructure");
        assert!(q.quality_score < 0.5);
    }

    #[test]
    fn test_low_quality_vague_scale_significant() {
        let q = validate_impact("Achieved significant performance improvements");
        assert!(q.quality_score < 0.5);
        assert!(q.flags.iter().any(|f| f.starts_with("vague_scale:")));
    }

    #[test]
    fn test_low_quality_major() {
        let q = validate_impact("Led major improvements to the codebase");
        assert!(q.quality_score < 0.5);
    }

    #[test]
    fn test_low_quality_various() {
        let q = validate_impact("Led various projects across teams");
        assert!(q.quality_score < 0.5);
    }

    #[test]
    fn test_medium_quality_no_metrics_clean_language() {
        // "Architected the authentication system" — no vague verbs, no numbers
        let q = validate_impact("Architected the authentication system");
        assert!(q.quality_score >= 0.4);
        assert!(q.flags.contains(&"missing_metric".to_string()));
        assert!(!q.suggestions.is_empty());
    }

    #[test]
    fn test_medium_quality_collaborated() {
        let q = validate_impact("Collaborated on the platform migration");
        // "collaborated" is not in the vague verbs list → medium quality
        assert!(q.quality_score >= 0.4);
    }

    #[test]
    fn test_validate_bullets_aggregate() {
        let bullets = vec![
            "Reduced latency by 40%".to_string(),
            "Improved the user experience".to_string(),
        ];
        let q = validate_bullets(&bullets);
        // avg of 1.0 and 0.4 = 0.7
        assert!(q.quality_score > 0.5 && q.quality_score < 1.0);
        assert!(!q.flags.is_empty());
    }

    #[test]
    fn test_validate_bullets_all_high() {
        let bullets = vec![
            "Reduced latency by 40%".to_string(),
            "Processed 100k records [LOW_METRICS]".to_string(),
        ];
        let q = validate_bullets(&bullets);
        assert_eq!(q.quality_score, 1.0);
        assert!(q.flags.is_empty());
    }

    #[test]
    fn test_validate_bullets_empty() {
        let q = validate_bullets(&[]);
        assert_eq!(q.quality_score, 1.0);
    }
}
