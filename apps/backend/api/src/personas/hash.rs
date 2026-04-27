use sha2::{Digest, Sha256};

use crate::models::resume::PersonaRow;

/// SHA-256 over sorted persona IDs.
/// Detects any add/remove of personas so the suggest endpoint can skip the LLM on cache hits.
pub fn compute_persona_hash(personas: &[PersonaRow]) -> String {
    let mut ids: Vec<String> = personas.iter().map(|p| p.id.to_string()).collect();
    ids.sort();
    let mut hasher = Sha256::new();
    hasher.update(ids.join(",").as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::resume::PersonaRow;
    use uuid::Uuid;

    fn make_persona(id: Uuid) -> PersonaRow {
        PersonaRow {
            id,
            user_id: Uuid::new_v4(),
            name: "Test".to_string(),
            emphasized_tags: vec![],
            suppressed_tags: vec![],
            tone_preference: None,
            section_order: None,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn hash_is_order_independent() {
        let id1 = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let id2 = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        let a = vec![make_persona(id1), make_persona(id2)];
        let b = vec![make_persona(id2), make_persona(id1)];
        assert_eq!(compute_persona_hash(&a), compute_persona_hash(&b));
    }

    #[test]
    fn empty_slice_does_not_panic() {
        let h = compute_persona_hash(&[]);
        assert!(!h.is_empty());
    }

    #[test]
    fn different_ids_produce_different_hash() {
        let id1 = Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap();
        let id2 = Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap();
        assert_ne!(
            compute_persona_hash(&[make_persona(id1)]),
            compute_persona_hash(&[make_persona(id2)])
        );
    }
}
