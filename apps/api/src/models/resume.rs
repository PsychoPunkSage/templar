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
