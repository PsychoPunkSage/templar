#![allow(dead_code)]

//! HTTP handlers for the Grounding API.
//!
//! GET /api/v1/resumes/:id/audit → returns the AuditManifest for a resume.

use axum::{
    extract::{Path, State},
    Json,
};
use uuid::Uuid;

use crate::errors::AppError;
use crate::grounding::manifest::manifest_from_bullet_rows;
use crate::grounding::types::AuditManifest;
use crate::models::resume::{ResumeBulletRow, ResumeRow};
use crate::state::AppState;

/// GET /api/v1/resumes/:id/audit
///
/// Returns the full audit manifest for a generated resume.
/// Reconstructs from persisted DB rows — does NOT re-score via LLM.
///
/// Responses:
/// - 200 OK + AuditManifest JSON
/// - 404 Not Found if the resume_id doesn't exist
pub async fn handle_get_audit_manifest(
    State(state): State<AppState>,
    Path(resume_id): Path<Uuid>,
) -> Result<Json<AuditManifest>, AppError> {
    // Step 1: Verify resume exists
    let _resume = sqlx::query_as::<_, ResumeRow>("SELECT * FROM resumes WHERE id = $1")
        .bind(resume_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Resume {resume_id} not found")))?;

    // Step 2: Load all bullets for this resume
    let bullets = sqlx::query_as::<_, ResumeBulletRow>(
        "SELECT * FROM resume_bullets WHERE resume_id = $1 ORDER BY section, id",
    )
    .bind(resume_id)
    .fetch_all(&state.db)
    .await?;

    // Step 3: Build manifest from persisted rows
    let manifest = manifest_from_bullet_rows(resume_id, &bullets);

    Ok(Json(manifest))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests (integration — require live DB, skip in unit test runs)
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // Integration tests for handle_get_audit_manifest.
    // These require a live PostgreSQL database and are skipped in unit test runs.

    #[tokio::test]
    #[ignore = "requires live database"]
    async fn test_get_audit_manifest_404_for_unknown_resume() {
        // Would: POST to test server → GET /api/v1/resumes/{random_id}/audit → expect 404
    }

    #[tokio::test]
    #[ignore = "requires live database"]
    async fn test_get_audit_manifest_returns_valid_json() {
        // Would: create a resume with bullets → GET /api/v1/resumes/{id}/audit → expect 200
    }

    #[tokio::test]
    #[ignore = "requires live database"]
    async fn test_get_audit_manifest_includes_all_bullets() {
        // Would: create resume with N bullets → verify manifest.entries.len() == N
    }
}
