use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Universal event envelope — every analytics event from every adapter must conform to this shape.
/// Produced by both the web SDK (TypeScript) and the API SDK (Rust).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEnvelope {
    /// UUID v7 (time-ordered). Client-generated. Idempotency key — server deduplicates on this.
    pub event_id: Uuid,

    /// Clerk user ID. None for unauthenticated sessions.
    pub user_id: Option<Uuid>,

    /// Generated on tab open, stored in sessionStorage. Resets on tab close.
    pub session_id: Uuid,

    /// Persistent cookie in localStorage. Survives across sessions and pre-login.
    pub anon_id: Uuid,

    /// "prod" | "dev" | "test" | "bot". Mandatory. dev/test events excluded from aggregations.
    pub source: String,

    /// "web" | "api". Set by the SDK, not the caller.
    pub service: String,

    /// Domain name: "resume_editor", "interview_prep", "cover_letter", "generation_api".
    pub adapter: String,

    /// Namespaced "domain.object.verb". Only registered types accepted.
    pub event_type: String,

    /// "1.0". Mandatory. Enables payload evolution without breaking queries.
    pub schema_version: String,

    /// When the user performed the action. Set by the client SDK.
    pub client_ts: DateTime<Utc>,

    /// Set by the analytics service on ingestion. Null in transit.
    pub server_ts: Option<DateTime<Utc>>,

    /// Auto-attached by the SDK from current app state (resume_id, project_id, etc).
    #[serde(default)]
    pub context: serde_json::Value,

    /// Adapter-specific payload. Schema-validated against the event registry.
    #[serde(default)]
    pub properties: serde_json::Value,
}

// ── Event type constants ─────────────────────────────────────────────────────
// Generation domain
pub const RESUME_GENERATION_STARTED: &str = "resume.generation.started";
pub const RESUME_GENERATION_COMPLETED: &str = "resume.generation.completed";
pub const RESUME_GENERATION_FAILED: &str = "resume.generation.failed";
pub const RESUME_REGENERATED: &str = "resume.regenerated";

// Editor domain
pub const RESUME_BULLET_EDITED: &str = "resume.bullet.edited";
pub const RESUME_BULLET_DELETED: &str = "resume.bullet.deleted";
pub const RESUME_SECTION_REORDERED: &str = "resume.section.reordered";
pub const RESUME_TEMPLATE_CHANGED: &str = "resume.template.changed";
pub const RESUME_EXPORTED: &str = "resume.exported";

// Interview prep domain
pub const INTERVIEW_OPENED: &str = "interview.opened";
pub const INTERVIEW_QUESTION_EXPANDED: &str = "interview.question.expanded";
pub const INTERVIEW_SESSION_DURATION: &str = "interview.session.duration";

// Cover letter domain
pub const COVERLETTER_GENERATED: &str = "coverletter.generated";
pub const COVERLETTER_REGENERATED: &str = "coverletter.regenerated";
pub const COVERLETTER_EXPORTED: &str = "coverletter.exported";

// Identity domain
pub const USER_IDENTIFIED: &str = "user.identified";
