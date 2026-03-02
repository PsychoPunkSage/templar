// Phase 5: Grounding / Anti-Hallucination System
// Implements: grounding scorer, scope inflation check, audit manifest generation.
// Hard constraint: no bullet with score < 0.80 is ever shown to the user.

#![allow(unused_imports)]

pub mod handlers;
pub mod manifest;
pub mod prompts;
pub mod scope_check;
pub mod scorer;
pub mod types;

// Re-exports for consumers (Phase 6+ will use these)
pub use manifest::{build_audit_manifest, manifest_from_bullet_rows};
pub use scope_check::check_scope_inflation;
pub use scorer::{regenerate_single_bullet, score_bullet};
pub use types::{AuditEntry, AuditManifest, GroundingResult, GroundingScore, GroundingVerdict};
