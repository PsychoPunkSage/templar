pub mod generator;
pub mod handlers;
pub mod prompts;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

// ────────────────────────────────────────────────────────────────────────────
// Public types
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverLetterTone {
    Formal,
    Conversational,
}

impl CoverLetterTone {
    pub fn as_instruction(&self) -> &'static str {
        match self {
            CoverLetterTone::Formal => {
                "formal business letter — precise, professional, third-person reserved"
            }
            CoverLetterTone::Conversational => {
                "warm and conversational — personable, first-person, genuine enthusiasm"
            }
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            CoverLetterTone::Formal => "formal",
            CoverLetterTone::Conversational => "conversational",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverLetterFocus {
    Technical,
    Leadership,
    CultureFit,
}

impl CoverLetterFocus {
    pub fn as_instruction(&self) -> &'static str {
        match self {
            CoverLetterFocus::Technical => {
                "emphasize technical depth, system design, and engineering achievements"
            }
            CoverLetterFocus::Leadership => {
                "emphasize leadership impact, team outcomes, and strategic influence"
            }
            CoverLetterFocus::CultureFit => {
                "emphasize cultural alignment, collaboration, values, and growth mindset"
            }
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            CoverLetterFocus::Technical => "technical",
            CoverLetterFocus::Leadership => "leadership",
            CoverLetterFocus::CultureFit => "culture_fit",
        }
    }
}

/// A single paragraph in the cover letter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CoverLetterParagraph {
    /// "hook" | "fit" | "culture" | "close"
    pub role: String,
    pub text: String,
}

/// Request body for POST /api/v1/cover-letters/generate.
#[derive(Debug, Deserialize)]
pub struct GenerateCoverLetterRequest {
    pub user_id: Uuid,
    pub jd_text: String,
    pub tone: CoverLetterTone,
    pub focus: CoverLetterFocus,
    pub resume_id: Option<Uuid>,
    pub persona_id: Option<Uuid>,
}

/// Internal LLM output schema for cover letter generation.
#[derive(Debug, Deserialize)]
pub struct CoverLetterLlmOutput {
    pub company_name: String,
    pub role_title: String,
    pub hook: String,
    pub fit: String,
    pub culture: String,
    pub close: String,
}

/// A row from the `cover_letters` table.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CoverLetterRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub resume_id: Option<Uuid>,
    pub persona_id: Option<Uuid>,
    pub jd_text_hash: String,
    pub tone: String,
    pub focus: String,
    pub content: serde_json::Value,
    pub company_name: Option<String>,
    pub role_title: Option<String>,
    pub created_at: DateTime<Utc>,
}
