#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ResumeRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub jd_text: String,
    pub jd_parsed: Option<Value>,
    pub fit_score: Option<f64>,
    pub latex_source: Option<String>,
    pub s3_pdf_key: Option<String>,
    pub status: String,
    /// Added in migration 004: which file-based template was used (None = legacy font template).
    /// TEXT column referencing the templates directory name, not a FK.
    pub template_id: Option<String>,
    /// Added in migration 009: SHA-256 hash of render inputs (template + profile + bullets).
    /// Used for cache-hit detection — if unchanged, skip re-render.
    pub content_hash: Option<String>,
    /// Added in migration 014: serialized Vec<EntryGroup> for page-reload restoration.
    /// NULL for resumes generated before this migration.
    pub entry_groups: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ResumeBulletRow {
    pub id: Uuid,
    pub resume_id: Uuid,
    pub section: String,
    pub bullet_text: String,
    pub source_entry_id: Uuid,
    pub grounding_score: f64,
    pub is_user_edited: bool,
    pub line_count: i16,
    pub rejection_reason: Option<String>,
    /// Added in migration 010: pre-formatted LaTeX entry header from LLM.
    /// Non-NULL only for the first bullet of each source_entry_id group.
    pub entry_header: Option<String>,
    /// Added in migration 011: insertion rank from the generation pipeline (0-based).
    /// Used in ORDER BY for render to preserve relevance-ranked order.
    pub order_idx: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RenderJobRow {
    pub id: Uuid,
    pub resume_id: Uuid,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A row from the `generation_jobs` table.
///
/// Used by the generation worker and the status endpoint.
/// `request` stores the full `GenerateRequest` payload as JSONB so the worker
/// can reconstruct the request without the HTTP connection being in scope.
/// `result` stores the full `GenerateResponse` payload on success so the status
/// endpoint can return `entry_groups` and `fit_report` without re-querying bullets.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GenerationJobRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub request: Value,
    pub status: String,
    pub error: Option<String>,
    pub resume_id: Option<Uuid>,
    pub result: Option<Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PersonaRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub emphasized_tags: Vec<String>,
    pub suppressed_tags: Vec<String>,
    pub tone_preference: Option<String>,
    pub section_order: Option<Value>,
    pub created_at: DateTime<Utc>,
}
