use std::collections::HashSet;

use crate::models::resume::PersonaRow;
use crate::personas::PersonaSuggestion;

/// Jaccard similarity between two tag sets (0.0 – 1.0).
/// Empty-vs-empty is treated as 1.0 (identical).
pub fn tag_jaccard(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let set_a: HashSet<&str> = a.iter().map(|s| s.as_str()).collect();
    let set_b: HashSet<&str> = b.iter().map(|s| s.as_str()).collect();
    let intersection = set_a.intersection(&set_b).count();
    let union = set_a.union(&set_b).count();
    intersection as f64 / union as f64
}

/// True if the suggestion is semantically equivalent to any existing persona.
///
/// Two checks:
///   1. Name containment — "Senior C++ Dev" ≈ "C++ Dev" (catches reworded titles).
///   2. Jaccard tag overlap > `threshold` — catches same-role suggestions with different names.
pub fn is_semantic_duplicate(
    s: &PersonaSuggestion,
    existing: &[PersonaRow],
    threshold: f64,
) -> bool {
    let s_name = s.name.to_lowercase();
    for p in existing {
        let p_name = p.name.to_lowercase();
        if s_name.contains(&p_name) || p_name.contains(&s_name) {
            return true;
        }
        if tag_jaccard(&s.emphasized_tags, &p.emphasized_tags) > threshold {
            return true;
        }
    }
    false
}

/// Filter out suggestions that duplicate existing personas.
pub fn dedup_suggestions(
    suggestions: Vec<PersonaSuggestion>,
    existing: &[PersonaRow],
    threshold: f64,
) -> Vec<PersonaSuggestion> {
    suggestions
        .into_iter()
        .filter(|s| !is_semantic_duplicate(s, existing, threshold))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::resume::PersonaRow;
    use uuid::Uuid;

    fn persona(name: &str, tags: &[&str]) -> PersonaRow {
        PersonaRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            name: name.to_string(),
            emphasized_tags: tags.iter().map(|s| s.to_string()).collect(),
            suppressed_tags: vec![],
            tone_preference: None,
            section_order: None,
            created_at: chrono::Utc::now(),
        }
    }

    fn suggestion(name: &str, tags: &[&str]) -> PersonaSuggestion {
        PersonaSuggestion {
            name: name.to_string(),
            emphasized_tags: tags.iter().map(|s| s.to_string()).collect(),
            suppressed_tags: vec![],
            tone_preference: None,
            reasoning: String::new(),
        }
    }

    #[test]
    fn jaccard_identical_sets() {
        let a = vec!["rust".to_string(), "systems".to_string()];
        assert!((tag_jaccard(&a, &a) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn jaccard_disjoint_sets() {
        let a = vec!["rust".to_string()];
        let b = vec!["python".to_string()];
        assert!((tag_jaccard(&a, &b) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn jaccard_partial_overlap() {
        let a = vec!["rust".to_string(), "cpp".to_string(), "perf".to_string()];
        let b = vec![
            "rust".to_string(),
            "cpp".to_string(),
            "leadership".to_string(),
        ];
        // intersection = 2, union = 4 → 0.5
        assert!((tag_jaccard(&a, &b) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn name_containment_detected() {
        let existing = vec![persona("C++ Dev", &["cpp"])];
        let s = suggestion("Senior C++ Dev", &["cpp", "leadership"]);
        assert!(is_semantic_duplicate(&s, &existing, 0.6));
    }

    #[test]
    fn high_jaccard_detected() {
        let existing = vec![persona("Systems Engineer", &["cpp", "systems", "perf"])];
        // 3/3 = 1.0 Jaccard
        let s = suggestion("Perf Engineer", &["cpp", "systems", "perf"]);
        assert!(is_semantic_duplicate(&s, &existing, 0.6));
    }

    #[test]
    fn distinct_suggestion_passes_through() {
        let existing = vec![persona("ML Engineer", &["ml", "python", "tensorflow"])];
        let s = suggestion("Rust Backend", &["rust", "axum", "postgres"]);
        assert!(!is_semantic_duplicate(&s, &existing, 0.6));
    }

    #[test]
    fn dedup_filters_and_preserves() {
        let existing = vec![persona("ML Engineer", &["ml", "python"])];
        let suggestions = vec![
            suggestion("ML Engineer", &["ml", "python"]), // exact name → filtered
            suggestion("Rust Backend", &["rust", "axum"]), // distinct → kept
        ];
        let result = dedup_suggestions(suggestions, &existing, 0.6);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].name, "Rust Backend");
    }
}
