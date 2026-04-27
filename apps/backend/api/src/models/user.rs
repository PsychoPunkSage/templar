#![allow(dead_code)]

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct User {
    pub id: Uuid,
    pub external_id: String,
    pub email: String,
    pub tier: String,
    pub created_at: DateTime<Utc>,
}

/// A single link entry in a user's profile (stored as JSONB array element).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileLink {
    /// "LinkedIn" | "GitHub" | "GitLab" | "Twitter" | "Portfolio" | "Custom"
    #[serde(rename = "type")]
    pub link_type: String,
    /// Display label — required when link_type is "Custom"
    pub label: Option<String>,
    pub url: String,
    /// Short alias shown in PDF header instead of raw URL (e.g. "PsychoPunkSage")
    pub alias: Option<String>,
}

/// User profile row from `user_profiles` table.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct UserProfile {
    pub id: Uuid,
    pub user_id: Uuid,
    pub full_name: String,
    pub email: String,
    pub phone: String,
    pub location: String,
    /// Raw JSONB value — callers use serde_json::from_value::<Vec<ProfileLink>>(row.links)
    pub links: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}
