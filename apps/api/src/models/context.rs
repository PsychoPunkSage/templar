use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ContextEntryRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub entry_id: Uuid,
    pub version: i32,
    pub entry_type: String,
    pub data: Value,
    pub raw_text: Option<String>,
    pub recency_score: f64,
    pub impact_score: f64,
    pub tags: Vec<String>,
    pub flagged_evergreen: bool,
    pub contribution_type: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct ContextSnapshotRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub version: i32,
    pub s3_key: String,
    pub created_at: DateTime<Utc>,
}
