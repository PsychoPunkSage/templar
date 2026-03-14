#![allow(dead_code)]
//! Axum route handlers for the Render API (Phase 4).
//!
//! POST /api/v1/render            → handle_trigger_render
//! GET  /api/v1/render/:job_id/status → handle_render_status
//! GET  /api/v1/render/:job_id    → handle_get_pdf

use axum::{
    body::Body,
    extract::{Path, State},
    http::{Response, StatusCode},
    Json,
};
use chrono::{DateTime, Utc};
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::errors::AppError;
use crate::models::resume::RenderJobRow;
use crate::render::worker::RENDER_QUEUE_KEY;
use crate::state::AppState;

// ────────────────────────────────────────────────────────────────────────────
// Request / Response types
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct TriggerRenderRequest {
    pub resume_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct TriggerRenderResponse {
    pub job_id: Uuid,
    pub status: String,
}

#[derive(Debug, Serialize)]
pub struct RenderStatusResponse {
    pub job_id: Uuid,
    pub resume_id: Uuid,
    pub status: String,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// ────────────────────────────────────────────────────────────────────────────
// Handlers
// ────────────────────────────────────────────────────────────────────────────

/// POST /api/v1/render
///
/// Triggers a render job for an existing resume.
/// Idempotent: if a queued or processing job already exists for the resume,
/// returns the existing job_id instead of creating a duplicate.
pub async fn handle_trigger_render(
    State(state): State<AppState>,
    Json(req): Json<TriggerRenderRequest>,
) -> Result<Json<TriggerRenderResponse>, AppError> {
    // Validate resume exists
    let resume_exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM resumes WHERE id = $1)")
            .bind(req.resume_id)
            .fetch_one(&state.db)
            .await?;

    if !resume_exists {
        return Err(AppError::NotFound(format!(
            "Resume {} not found",
            req.resume_id
        )));
    }

    // Idempotency: return existing active job if one exists.
    // The `updated_at > NOW() - INTERVAL '5 minutes'` guard prevents stale
    // 'processing' jobs (e.g. a worker that crashed mid-job and never transitioned
    // to 'failed') from blocking new render requests indefinitely.
    let existing = sqlx::query_as::<_, RenderJobRow>(
        "SELECT * FROM render_jobs \
         WHERE resume_id = $1 AND status IN ('queued', 'processing') \
         AND updated_at > NOW() - INTERVAL '5 minutes' \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(req.resume_id)
    .fetch_optional(&state.db)
    .await?;

    if let Some(job) = existing {
        return Ok(Json(TriggerRenderResponse {
            job_id: job.id,
            status: job.status,
        }));
    }

    // Insert new render job row
    let job_id = Uuid::new_v4();
    sqlx::query("INSERT INTO render_jobs (id, resume_id, status) VALUES ($1, $2, 'queued')")
        .bind(job_id)
        .bind(req.resume_id)
        .execute(&state.db)
        .await?;

    // Push job_id to Redis queue (async)
    let mut conn = state
        .redis
        .get_multiplexed_async_connection()
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Redis connection failed: {e}")))?;

    conn.lpush::<_, _, ()>(RENDER_QUEUE_KEY, job_id.to_string())
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Redis enqueue failed: {e}")))?;

    tracing::info!(
        "Triggered render job {} for resume {}",
        job_id,
        req.resume_id
    );

    Ok(Json(TriggerRenderResponse {
        job_id,
        status: "queued".to_string(),
    }))
}

/// GET /api/v1/render/:job_id/status
///
/// Returns the current status of a render job.
pub async fn handle_render_status(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Json<RenderStatusResponse>, AppError> {
    let job = sqlx::query_as::<_, RenderJobRow>("SELECT * FROM render_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Render job {job_id} not found")))?;

    Ok(Json(RenderStatusResponse {
        job_id: job.id,
        resume_id: job.resume_id,
        status: job.status,
        error_message: job.error_message,
        created_at: job.created_at,
        updated_at: job.updated_at,
    }))
}

/// GET /api/v1/render/:job_id
///
/// Returns the rendered PDF for a completed render job.
///
/// Returns 409 Conflict if the job is not yet done.
/// Returns the PDF as an `application/pdf` response body.
pub async fn handle_get_pdf(
    State(state): State<AppState>,
    Path(job_id): Path<Uuid>,
) -> Result<Response<Body>, AppError> {
    // Fetch job record
    let job = sqlx::query_as::<_, RenderJobRow>("SELECT * FROM render_jobs WHERE id = $1")
        .bind(job_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Render job {job_id} not found")))?;

    // Only serve PDF if job is done
    if job.status != "done" {
        let body = Body::from(
            serde_json::to_vec(&serde_json::json!({
                "error": {
                    "code": "RENDER_NOT_READY",
                    "message": format!("Render job {} is not complete (status={})", job_id, job.status)
                }
            }))
            .unwrap_or_default(),
        );
        return Response::builder()
            .status(StatusCode::CONFLICT)
            .header("Content-Type", "application/json")
            .body(body)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("Response build error: {e}")));
    }

    // Fetch S3 key from resume
    let s3_pdf_key: Option<String> =
        sqlx::query_scalar("SELECT s3_pdf_key FROM resumes WHERE id = $1")
            .bind(job.resume_id)
            .fetch_optional(&state.db)
            .await?
            .flatten();

    let s3_key = s3_pdf_key.ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "Render job {} done but resume {} has no s3_pdf_key",
            job_id,
            job.resume_id
        ))
    })?;

    // Fetch PDF from S3
    let s3_response = state
        .s3
        .get_object()
        .bucket(&state.config.s3_bucket)
        .key(&s3_key)
        .send()
        .await
        .map_err(|e| AppError::S3(format!("Failed to fetch PDF from S3: {e}")))?;

    let bytes = s3_response
        .body
        .collect()
        .await
        .map_err(|e| AppError::S3(format!("Failed to read PDF stream from S3: {e}")))?
        .into_bytes();

    let content_length = bytes.len().to_string();

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "application/pdf")
        .header("Content-Length", content_length)
        .header(
            "Content-Disposition",
            format!("attachment; filename=\"resume-{}.pdf\"", job.resume_id),
        )
        .body(Body::from(bytes.to_vec()))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("Failed to build PDF response: {e}")))
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    // Handler tests require a live DB + Redis.
    // These are integration tests marked #[ignore] for unit test runs.

    /// Integration test — requires live DB and Redis.
    #[tokio::test]
    #[ignore]
    async fn test_trigger_render_returns_404_for_unknown_resume() {
        // Would build a test AppState + axum::test::TestClient, POST with a random resume_id,
        // and assert 404 response.
    }

    /// Integration test — requires live DB.
    #[tokio::test]
    #[ignore]
    async fn test_render_status_returns_404_for_unknown_job() {
        // Would GET /api/v1/render/{random_uuid}/status and assert 404.
    }

    /// Integration test — requires live DB.
    #[tokio::test]
    #[ignore]
    async fn test_get_pdf_returns_409_when_not_done() {
        // Would insert a render_jobs row with status='queued', GET the PDF endpoint,
        // and assert 409 Conflict.
    }

    /// Integration test — requires live DB and Redis.
    #[tokio::test]
    #[ignore]
    async fn test_trigger_render_is_idempotent() {
        // Would trigger render twice for the same resume and verify the same job_id
        // is returned on the second call.
    }

    /// Integration test — requires live DB.
    #[tokio::test]
    #[ignore]
    async fn test_trigger_render_validates_resume_exists() {
        // Would POST with a non-existent resume_id and assert 404.
    }
}
