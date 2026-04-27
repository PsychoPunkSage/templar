#![allow(dead_code)]

//! Grounding types — GroundingScore, GroundingResult, GroundingVerdict, AuditEntry, AuditManifest.
//!
//! Every resume bullet must carry a composite grounding score ≥ 0.80 to pass.
//! Bullets between 0.65–0.80 are flagged for review; below 0.65 are rejected.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ─────────────────────────────────────────────────────────────────────────────
// Grounding score
// ─────────────────────────────────────────────────────────────────────────────

/// Four-component grounding score for a single resume bullet.
///
/// Composite formula:
///   composite = 0.40 * source_match + 0.30 * specificity_fidelity + 0.20 * scope_accuracy - 0.10 * interpolation_risk
///
/// All component scores are in [0.0, 1.0].
/// interpolation_risk is a *penalty* — higher interpolation risk lowers the composite.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroundingScore {
    /// Weight 0.40 — does a context entry directly support this bullet?
    pub source_match: f32,
    /// Weight 0.30 — are numbers/tools taken directly from context, not inferred?
    pub specificity_fidelity: f32,
    /// Weight 0.20 — is the claimed ownership/impact scope accurate?
    pub scope_accuracy: f32,
    /// Weight 0.10 — penalty if the LLM invented details not in context.
    pub interpolation_risk: f32,
    /// Final composite score.
    pub composite: f32,
}

impl GroundingScore {
    /// Computes the composite score from the four components.
    ///
    /// composite = 0.40 * source_match + 0.30 * specificity + 0.20 * scope - 0.10 * interp
    /// Clamped to [0.0, 1.0].
    pub fn compute(source_match: f32, specificity: f32, scope: f32, interp: f32) -> Self {
        let composite = (0.40 * source_match + 0.30 * specificity + 0.20 * scope - 0.10 * interp)
            .clamp(0.0, 1.0);
        Self {
            source_match,
            specificity_fidelity: specificity,
            scope_accuracy: scope,
            interpolation_risk: interp,
            composite,
        }
    }

    /// Returns the verdict for this score.
    pub fn verdict(&self) -> GroundingVerdict {
        GroundingVerdict::from_composite(self.composite)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Verdict
// ─────────────────────────────────────────────────────────────────────────────

/// The outcome of grounding evaluation for a single bullet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GroundingVerdict {
    /// composite ≥ 0.80 — bullet is grounded and safe to show to the user.
    Pass,
    /// 0.65 ≤ composite < 0.80 — bullet is included but flagged for user review.
    FlagForReview,
    /// composite < 0.65 — bullet is rejected; will be regenerated or omitted.
    Fail,
}

impl GroundingVerdict {
    /// Derives the verdict from a composite score.
    pub fn from_composite(composite: f32) -> Self {
        if composite >= 0.80 {
            Self::Pass
        } else if composite >= 0.65 {
            Self::FlagForReview
        } else {
            Self::Fail
        }
    }

    /// Returns the serialized string representation for DB storage / audit.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::FlagForReview => "flag_for_review",
            Self::Fail => "fail",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Grounding result
// ─────────────────────────────────────────────────────────────────────────────

/// The complete grounding evaluation result for one bullet.
#[derive(Debug, Clone, Serialize)]
pub struct GroundingResult {
    pub bullet_text: String,
    pub source_entry_id: Uuid,
    pub score: GroundingScore,
    pub verdict: GroundingVerdict,
    /// Set when scope inflation is detected or the LLM flags interpolation.
    pub rejection_reason: Option<String>,
}

impl GroundingResult {
    /// Constructs a synthetic fail result for scope inflation (no LLM call needed).
    pub fn scope_inflation_fail(
        bullet_text: String,
        source_entry_id: Uuid,
        reason: String,
    ) -> Self {
        let score = GroundingScore::compute(
            0.5, // source_match — likely still references the right entry
            0.5, // specificity_fidelity — data may be correct
            0.2, // scope_accuracy — low because scope is inflated
            0.8, // interpolation_risk — high, language not from context
        );
        Self {
            bullet_text,
            source_entry_id,
            verdict: GroundingVerdict::Fail,
            score,
            rejection_reason: Some(reason),
        }
    }

    /// Constructs a fail-safe result when the grounding LLM call errors.
    /// Returns FlagForReview (not Fail) so generation is never blocked entirely.
    pub fn llm_error_fallback(bullet_text: String, source_entry_id: Uuid) -> Self {
        let score = GroundingScore::compute(0.7, 0.7, 0.7, 0.2); // composite ≈ 0.70
        Self {
            bullet_text,
            source_entry_id,
            verdict: GroundingVerdict::FlagForReview,
            score,
            rejection_reason: Some(
                "Grounding scorer unavailable — flagged for human review".to_string(),
            ),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Audit types
// ─────────────────────────────────────────────────────────────────────────────

/// One entry in the audit manifest — one per bullet in the resume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub bullet_text: String,
    pub source_entry_id: Uuid,
    pub composite_score: f32,
    /// Serialized verdict: "pass" | "flag_for_review" | "fail"
    pub verdict: String,
    pub rejection_reason: Option<String>,
    pub section: String,
}

/// Complete audit manifest for a generated resume.
///
/// Returned by `GET /api/v1/resumes/:id/audit`.
#[derive(Debug, Serialize, Deserialize)]
pub struct AuditManifest {
    pub resume_id: Uuid,
    pub generated_at: DateTime<Utc>,
    pub entries: Vec<AuditEntry>,
    /// Fraction of bullets that passed (Pass verdict / total).
    pub overall_pass_rate: f32,
    /// Count of bullets with Fail verdict.
    pub bullets_rejected: u32,
    /// Count of bullets with FlagForReview verdict.
    pub bullets_flagged: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn test_grounding_score_composite_formula() {
        // composite = 0.40*0.9 + 0.30*0.8 + 0.20*0.85 - 0.10*0.1
        //           = 0.36 + 0.24 + 0.17 - 0.01 = 0.76
        let score = GroundingScore::compute(0.9, 0.8, 0.85, 0.1);
        let expected = 0.40 * 0.9 + 0.30 * 0.8 + 0.20 * 0.85 - 0.10 * 0.1;
        assert!(
            (score.composite - expected).abs() < 1e-5,
            "composite={} expected={}",
            score.composite,
            expected
        );
        assert_eq!(score.source_match, 0.9);
        assert_eq!(score.specificity_fidelity, 0.8);
        assert_eq!(score.scope_accuracy, 0.85);
        assert_eq!(score.interpolation_risk, 0.1);
    }

    #[test]
    fn test_grounding_verdict_thresholds() {
        // Pass ≥ 0.80
        assert_eq!(
            GroundingVerdict::from_composite(0.80),
            GroundingVerdict::Pass
        );
        assert_eq!(
            GroundingVerdict::from_composite(0.95),
            GroundingVerdict::Pass
        );
        assert_eq!(
            GroundingVerdict::from_composite(1.0),
            GroundingVerdict::Pass
        );

        // FlagForReview: 0.65 ≤ x < 0.80
        assert_eq!(
            GroundingVerdict::from_composite(0.65),
            GroundingVerdict::FlagForReview
        );
        assert_eq!(
            GroundingVerdict::from_composite(0.70),
            GroundingVerdict::FlagForReview
        );
        assert_eq!(
            GroundingVerdict::from_composite(0.799),
            GroundingVerdict::FlagForReview
        );

        // Fail < 0.65
        assert_eq!(
            GroundingVerdict::from_composite(0.64),
            GroundingVerdict::Fail
        );
        assert_eq!(
            GroundingVerdict::from_composite(0.0),
            GroundingVerdict::Fail
        );
    }

    #[test]
    fn test_audit_manifest_serializes_to_json() {
        let manifest = AuditManifest {
            resume_id: Uuid::new_v4(),
            generated_at: Utc::now(),
            entries: vec![AuditEntry {
                bullet_text: "Contributed to distributed caching layer".to_string(),
                source_entry_id: Uuid::new_v4(),
                composite_score: 0.88,
                verdict: "pass".to_string(),
                rejection_reason: None,
                section: "experience".to_string(),
            }],
            overall_pass_rate: 1.0,
            bullets_rejected: 0,
            bullets_flagged: 0,
        };

        let json = serde_json::to_string(&manifest).expect("manifest must serialize");
        assert!(json.contains("overall_pass_rate"));
        assert!(json.contains("bullets_rejected"));
        assert!(json.contains("bullets_flagged"));
        assert!(json.contains("pass"));

        // Round-trip
        let recovered: AuditManifest =
            serde_json::from_str(&json).expect("manifest must deserialize");
        assert_eq!(recovered.bullets_rejected, 0);
        assert_eq!(recovered.bullets_flagged, 0);
        assert!((recovered.overall_pass_rate - 1.0).abs() < 1e-5);
    }
}
