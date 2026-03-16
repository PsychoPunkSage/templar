use sha2::{Digest, Sha256};
use crate::models::context::ContextEntryRow;

/// Compute a stable SHA-256 hash for a job description string.
///
/// Normalises by trimming whitespace and lowercasing before hashing so that
/// cosmetic differences (leading/trailing spaces, case) produce identical hashes.
pub fn compute_jd_hash(jd_text: &str) -> String {
    let normalized = jd_text.trim().to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    hex::encode(hasher.finalize())
}

/// Compute a stable SHA-256 hash over a slice of context entries.
///
/// Entries are sorted by `entry_id` before hashing so the result is
/// independent of the order in which entries are returned from the DB.
/// Only `raw_text` is included in the hash — structured `data` fields are
/// intentionally excluded because they are derived from raw_text.
pub fn compute_context_hash(entries: &[ContextEntryRow]) -> String {
    let mut sorted = entries.to_vec();
    sorted.sort_by_key(|e| e.entry_id);
    let combined = sorted
        .iter()
        .map(|e| e.raw_text.as_deref().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n---\n");
    let mut hasher = Sha256::new();
    hasher.update(combined.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;
    use serde_json::Value;
    use crate::models::context::ContextEntryRow;

    fn make_entry(entry_id: Uuid, raw_text: &str) -> ContextEntryRow {
        ContextEntryRow {
            id: Uuid::new_v4(),
            entry_id,
            user_id: Uuid::new_v4(),
            version: 1,
            entry_type: "experience".to_string(),
            data: Value::Null,
            raw_text: Some(raw_text.to_string()),
            recency_score: 1.0,
            impact_score: 1.0,
            tags: vec![],
            flagged_evergreen: false,
            contribution_type: "sole_author".to_string(),
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_jd_hash_deterministic() {
        let h1 = compute_jd_hash("  Senior Rust Engineer  ");
        let h2 = compute_jd_hash("senior rust engineer");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_jd_hash_different_for_different_input() {
        let h1 = compute_jd_hash("Senior Rust Engineer");
        let h2 = compute_jd_hash("Junior Python Developer");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_context_hash_stable_across_order() {
        let id1 = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let id2 = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let entries_a = vec![make_entry(id1, "entry one"), make_entry(id2, "entry two")];
        let entries_b = vec![make_entry(id2, "entry two"), make_entry(id1, "entry one")];
        assert_eq!(compute_context_hash(&entries_a), compute_context_hash(&entries_b));
    }

    #[test]
    fn test_context_hash_none_raw_text() {
        // Entries with None raw_text should hash as empty string — no panic.
        let id1 = Uuid::parse_str("00000000-0000-0000-0000-000000000003").unwrap();
        let mut entry = make_entry(id1, "some text");
        entry.raw_text = None;
        // Should not panic
        let hash = compute_context_hash(&[entry]);
        assert!(!hash.is_empty());
    }
}
