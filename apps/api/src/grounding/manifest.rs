#![allow(dead_code)]

//! Audit manifest builder.
//!
//! Two construction paths:
//! 1. `build_audit_manifest()` — from fresh GroundingResult objects (during generation).
//! 2. `manifest_from_bullet_rows()` — reconstructed from DB rows (GET /audit endpoint).

use std::collections::HashMap;

use chrono::Utc;
use uuid::Uuid;

use crate::grounding::types::{AuditEntry, AuditManifest, GroundingResult, GroundingVerdict};
use crate::models::resume::ResumeBulletRow;

/// Builds an AuditManifest from fresh GroundingResult objects produced during generation.
///
/// `sections` maps source_entry_id → section name for filling the AuditEntry.section field.
pub fn build_audit_manifest(
    resume_id: Uuid,
    results: &[(GroundingResult, String)], // (result, section)
) -> AuditManifest {
    let total = results.len() as u32;
    let mut bullets_rejected = 0u32;
    let mut bullets_flagged = 0u32;
    let mut pass_count = 0u32;

    let entries: Vec<AuditEntry> = results
        .iter()
        .map(|(result, section)| {
            match result.verdict {
                GroundingVerdict::Pass => pass_count += 1,
                GroundingVerdict::FlagForReview => bullets_flagged += 1,
                GroundingVerdict::Fail => bullets_rejected += 1,
            }
            AuditEntry {
                bullet_text: result.bullet_text.clone(),
                source_entry_id: result.source_entry_id,
                composite_score: result.score.composite,
                verdict: result.verdict.as_str().to_string(),
                rejection_reason: result.rejection_reason.clone(),
                section: section.clone(),
            }
        })
        .collect();

    let overall_pass_rate = if total == 0 {
        1.0 // vacuously true — no bullets to reject
    } else {
        pass_count as f32 / total as f32
    };

    AuditManifest {
        resume_id,
        generated_at: Utc::now(),
        entries,
        overall_pass_rate,
        bullets_rejected,
        bullets_flagged,
    }
}

/// Reconstructs an AuditManifest from persisted DB rows (no LLM re-scoring).
///
/// Uses the stored `grounding_score` value to infer the verdict via the same
/// threshold constants used at scoring time (≥ 0.80 = Pass, ≥ 0.65 = FlagForReview).
pub fn manifest_from_bullet_rows(resume_id: Uuid, bullets: &[ResumeBulletRow]) -> AuditManifest {
    let total = bullets.len() as u32;
    let mut bullets_rejected = 0u32;
    let mut bullets_flagged = 0u32;
    let mut pass_count = 0u32;

    let entries: Vec<AuditEntry> = bullets
        .iter()
        .map(|row| {
            let composite = row.grounding_score as f32;
            let verdict = GroundingVerdict::from_composite(composite);
            let verdict_str = verdict.as_str().to_string();

            match verdict {
                GroundingVerdict::Pass => pass_count += 1,
                GroundingVerdict::FlagForReview => bullets_flagged += 1,
                GroundingVerdict::Fail => bullets_rejected += 1,
            }

            AuditEntry {
                bullet_text: row.bullet_text.clone(),
                source_entry_id: row.source_entry_id,
                composite_score: composite,
                verdict: verdict_str,
                rejection_reason: None, // not persisted in current schema
                section: row.section.clone(),
            }
        })
        .collect();

    let overall_pass_rate = if total == 0 {
        1.0
    } else {
        pass_count as f32 / total as f32
    };

    AuditManifest {
        resume_id,
        generated_at: Utc::now(),
        entries,
        overall_pass_rate,
        bullets_rejected,
        bullets_flagged,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    use crate::grounding::types::{GroundingScore, GroundingVerdict};
    use crate::models::resume::ResumeBulletRow;

    fn make_result(bullet: &str, composite: f32, section: &str) -> (GroundingResult, String) {
        let score = GroundingScore {
            source_match: composite,
            specificity_fidelity: composite,
            scope_accuracy: composite,
            interpolation_risk: 0.0,
            composite,
        };
        let verdict = GroundingVerdict::from_composite(composite);
        (
            GroundingResult {
                bullet_text: bullet.to_string(),
                source_entry_id: Uuid::new_v4(),
                score,
                verdict,
                rejection_reason: None,
            },
            section.to_string(),
        )
    }

    #[test]
    fn test_manifest_pass_rate_all_pass() {
        let results = vec![
            make_result("Bullet 1", 0.90, "experience"),
            make_result("Bullet 2", 0.85, "experience"),
            make_result("Bullet 3", 0.95, "skills"),
        ];
        let manifest = build_audit_manifest(Uuid::new_v4(), &results);
        assert!(
            (manifest.overall_pass_rate - 1.0).abs() < 1e-5,
            "all pass → pass_rate=1.0"
        );
        assert_eq!(manifest.bullets_rejected, 0);
        assert_eq!(manifest.bullets_flagged, 0);
        assert_eq!(manifest.entries.len(), 3);
    }

    #[test]
    fn test_manifest_counts_correctly() {
        let results = vec![
            make_result("Pass bullet", 0.90, "experience"), // pass
            make_result("Flag bullet", 0.72, "experience"), // flag_for_review
            make_result("Fail bullet", 0.50, "experience"), // fail
            make_result("Another pass", 0.85, "skills"),    // pass
        ];
        let manifest = build_audit_manifest(Uuid::new_v4(), &results);

        assert_eq!(
            manifest.bullets_rejected, 1,
            "one bullet should be rejected"
        );
        assert_eq!(manifest.bullets_flagged, 1, "one bullet should be flagged");

        // pass_count = 2 out of 4
        let expected_pass_rate = 2.0 / 4.0;
        assert!(
            (manifest.overall_pass_rate - expected_pass_rate).abs() < 1e-5,
            "pass_rate={} expected={}",
            manifest.overall_pass_rate,
            expected_pass_rate
        );
    }

    #[test]
    fn test_manifest_zero_bullets() {
        let results: Vec<(GroundingResult, String)> = vec![];
        let manifest = build_audit_manifest(Uuid::new_v4(), &results);
        assert!(
            (manifest.overall_pass_rate - 1.0).abs() < 1e-5,
            "empty → pass_rate=1.0"
        );
        assert_eq!(manifest.bullets_rejected, 0);
        assert_eq!(manifest.bullets_flagged, 0);
        assert!(manifest.entries.is_empty());
    }

    #[test]
    fn test_manifest_from_bullet_rows_reconstructs_correctly() {
        let resume_id = Uuid::new_v4();
        let now = Utc::now();

        let rows = vec![
            ResumeBulletRow {
                id: Uuid::new_v4(),
                resume_id,
                section: "experience".to_string(),
                bullet_text: "Contributed to caching layer".to_string(),
                source_entry_id: Uuid::new_v4(),
                grounding_score: 0.88, // pass
                is_user_edited: false,
                line_count: 1,
                created_at: now,
            },
            ResumeBulletRow {
                id: Uuid::new_v4(),
                resume_id,
                section: "experience".to_string(),
                bullet_text: "Built something great".to_string(),
                source_entry_id: Uuid::new_v4(),
                grounding_score: 0.70, // flag_for_review
                is_user_edited: false,
                line_count: 1,
                created_at: now,
            },
            ResumeBulletRow {
                id: Uuid::new_v4(),
                resume_id,
                section: "skills".to_string(),
                bullet_text: "Did something vague".to_string(),
                source_entry_id: Uuid::new_v4(),
                grounding_score: 0.40, // fail
                is_user_edited: false,
                line_count: 1,
                created_at: now,
            },
        ];

        let manifest = manifest_from_bullet_rows(resume_id, &rows);
        assert_eq!(manifest.resume_id, resume_id);
        assert_eq!(manifest.entries.len(), 3);
        assert_eq!(manifest.bullets_rejected, 1);
        assert_eq!(manifest.bullets_flagged, 1);

        let expected_pass_rate = 1.0 / 3.0;
        assert!(
            (manifest.overall_pass_rate - expected_pass_rate).abs() < 1e-5,
            "pass_rate={} expected={}",
            manifest.overall_pass_rate,
            expected_pass_rate
        );

        // Verify verdict strings are correctly assigned
        assert_eq!(manifest.entries[0].verdict, "pass");
        assert_eq!(manifest.entries[1].verdict, "flag_for_review");
        assert_eq!(manifest.entries[2].verdict, "fail");
    }
}
