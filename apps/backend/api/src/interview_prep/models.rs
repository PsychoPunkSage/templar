//! Data types for the interview prep subsystem.
//!
//! These types map 1:1 to the interview_prep_bullets and interview_prep_meta tables.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ────────────────────────────────────────────────────────────────────────────
// Status enum
// ────────────────────────────────────────────────────────────────────────────

/// Lifecycle status stored in interview_prep_meta.status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrepStatus {
    Pending,
    Generating,
    Ready,
    Failed,
}

impl PrepStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrepStatus::Pending => "pending",
            PrepStatus::Generating => "generating",
            PrepStatus::Ready => "ready",
            PrepStatus::Failed => "failed",
        }
    }
}

impl std::fmt::Display for PrepStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ────────────────────────────────────────────────────────────────────────────
// STAR scaffold
// ────────────────────────────────────────────────────────────────────────────

/// A STAR scaffold for a single resume bullet.
///
/// Every field must be grounded in the associated context entry — the LLM
/// must not invent facts absent from context.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StarScaffold {
    /// Background and situation the candidate was operating in.
    pub situation: String,
    /// The specific task or objective assigned/adopted.
    pub task: String,
    /// What the candidate actually did — concrete actions, tools used.
    pub action: String,
    /// Measurable outcome; numbers taken directly from context where available.
    pub result: String,
    /// 2–4 concise talking points the candidate should memorise; no dashes.
    #[serde(default)]
    pub talking_points: Vec<String>,
}

// ────────────────────────────────────────────────────────────────────────────
// Prep question types
// ────────────────────────────────────────────────────────────────────────────

/// Classification of an interview question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionType {
    Behavioral,
    Technical,
    RoleFit,
}

/// A single interview question linked to a resume bullet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepQuestion {
    pub text: String,
    #[serde(rename = "type")]
    pub question_type: QuestionType,
}

/// A gap question derived from FitReport.gaps — asks the candidate to address
/// areas where their context is weak against JD requirements.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GapQuestion {
    pub text: String,
    pub gap_area: String,
}

// ────────────────────────────────────────────────────────────────────────────
// Company context (optional refinement)
// ────────────────────────────────────────────────────────────────────────────

/// Optional company context that refines question tone and targeting.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CompanyContext {
    /// Company name — inferred from JD text or user-provided.
    pub company_name: String,
    /// Company stage/size (e.g. "Series B startup", "Fortune 500").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company_stage: Option<String>,
    /// Role title extracted from JD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role_title: Option<String>,
}

// ────────────────────────────────────────────────────────────────────────────
// HTTP response types
// ────────────────────────────────────────────────────────────────────────────

/// A fully-resolved prep bullet returned by the GET /:project_id endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepBullet {
    pub id: Uuid,
    pub project_id: Uuid,
    /// SHA-256 of trimmed lowercase bullet_text — used as a stable identifier.
    pub bullet_hash: String,
    /// Original bullet text (NOT normalised — display as-is).
    pub bullet_text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_entry_id: Option<Uuid>,
    pub star_scaffold: StarScaffold,
    pub questions: Vec<PrepQuestion>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Metadata for a project's interview prep session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepMeta {
    pub id: Uuid,
    pub project_id: Uuid,
    pub gap_questions: Vec<GapQuestion>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub company_context: Option<CompanyContext>,
    pub status: PrepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_generated_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Server-computed: true when the current resume bullets differ from the
    /// bullet hashes stored in interview_prep_bullets for this project.
    pub is_stale: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Full prep response returned by GET /:project_id.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<PrepMeta>,
    pub bullets: Vec<PrepBullet>,
}

/// Status-only response for GET /:project_id/status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrepStatusResponse {
    pub status: PrepStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_generated_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    pub is_stale: bool,
}

// ────────────────────────────────────────────────────────────────────────────
// DB row types (internal — sqlx FROM row)
// ────────────────────────────────────────────────────────────────────────────

/// Raw row from interview_prep_bullets — deserialized for DB operations.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PrepBulletRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub bullet_hash: String,
    pub bullet_text: String,
    pub context_entry_id: Option<Uuid>,
    pub star_scaffold: serde_json::Value,
    pub questions: serde_json::Value,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Raw row from interview_prep_meta.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PrepMetaRow {
    pub id: Uuid,
    pub project_id: Uuid,
    pub gap_questions: serde_json::Value,
    pub company_context: Option<serde_json::Value>,
    pub status: String,
    pub last_generated_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub is_stale: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ────────────────────────────────────────────────────────────────────────────
// LLM output schemas (internal)
// ────────────────────────────────────────────────────────────────────────────

/// LLM output for a single bullet's STAR scaffold + questions.
#[derive(Debug, Deserialize)]
pub struct BulletPrepLlmOutput {
    pub star_scaffold: StarScaffold,
    pub questions: Vec<PrepQuestion>,
}

/// LLM output for gap questions batch.
#[derive(Debug, Deserialize)]
pub struct GapQuestionsLlmOutput {
    pub gap_questions: Vec<GapQuestion>,
    /// Company name extracted from JD (opportunistic).
    #[serde(default)]
    pub company_name: Option<String>,
    /// Role title extracted from JD (opportunistic).
    #[serde(default)]
    pub role_title: Option<String>,
    /// Company stage inferred from JD (opportunistic).
    #[serde(default)]
    pub company_stage: Option<String>,
}
