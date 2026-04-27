//! CV Projects module — project-centric workspace management.
//!
//! A `cv_project` is the user-facing "workspace" concept: a named project
//! linking a template choice to the latest generated resume. Projects persist
//! independently of resumes — deleting a resume sets `current_resume_id` to NULL
//! but does not delete the project.
//!
//! This module is thin: it exposes types and re-exports handlers.
//! All business logic lives in handlers.rs.

pub mod handlers;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

// ────────────────────────────────────────────────────────────────────────────
// DB row type
// ────────────────────────────────────────────────────────────────────────────

/// A row from the `cv_projects` table.
///
/// Using runtime `query_as::<_, CvProjectRow>()` (not the macro `query_as!`)
/// so this type does not need an entry in the `.sqlx/` offline cache.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CvProjectRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub template_id: String,
    pub current_resume_id: Option<Uuid>,
    pub last_jd_text: Option<String>,
    /// Added in migration 013: tracks the most recent async generation job for this project.
    /// On page load, the editor probes this job's status endpoint to resume polling if in-flight.
    pub generation_job_id: Option<Uuid>,
    /// Added in migration 016: whether this project targets a single-page resume or a multi-page CV.
    /// Defaults to 'single_page' for backward compatibility.
    #[sqlx(default)]
    pub document_type: String,
    /// Added in migration 019: the most recently generated cover letter for this project.
    #[sqlx(default)]
    pub current_cover_letter_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ────────────────────────────────────────────────────────────────────────────
// Request / response types
// ────────────────────────────────────────────────────────────────────────────

/// Body for `POST /api/v1/projects`.
#[derive(Debug, Deserialize)]
pub struct CreateProjectRequest {
    pub user_id: Uuid,
    pub name: String,
    /// Must be a key in the loaded TemplateCache. Validated against cache (no DB query).
    pub template_id: String,
    /// Whether this project targets a single-page resume or a multi-page CV.
    /// Defaults to 'single_page' if not provided.
    pub document_type: Option<String>,
}

/// Body for `PATCH /api/v1/projects/:id` — all fields are optional.
#[derive(Debug, Deserialize)]
pub struct UpdateProjectRequest {
    pub name: Option<String>,
    pub template_id: Option<String>,
    pub current_resume_id: Option<Uuid>,
    pub last_jd_text: Option<String>,
    /// Set when a generation job is enqueued so polling can resume after page navigation.
    pub generation_job_id: Option<Uuid>,
}
