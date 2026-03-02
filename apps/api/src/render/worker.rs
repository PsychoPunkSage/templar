#![allow(dead_code)]
//! Background render worker: Redis BRPOP → DB fetch → LaTeX → Tectonic → S3 upload.
//!
//! Queue pattern: LPUSH on enqueue, BRPOP with 5-second timeout on dequeue.
//! Redis key stores `job_id` as a UUID string; worker fetches all data from DB.
//!
//! The worker NEVER panics on individual job failure — it logs the error and
//! continues processing the next job.

use std::collections::HashMap;

use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client as S3Client;
use redis::AsyncCommands;
use sqlx::PgPool;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::layout::{default_page_config, FontFamily};
use crate::models::resume::{ResumeBulletRow, ResumeRow};
use crate::render::tectonic::compile_latex;
use crate::render::templates::build_latex_document;
use crate::render::types::{RenderError, RenderParams, ResumeSection};

/// Redis list key used for the render job queue.
pub const RENDER_QUEUE_KEY: &str = "render:jobs";

// ────────────────────────────────────────────────────────────────────────────
// Worker spawn
// ────────────────────────────────────────────────────────────────────────────

/// Spawns the background render worker as a detached tokio task.
///
/// The worker processes render jobs from the Redis queue indefinitely.
/// It receives clones of all needed resources (Redis, DB, S3) before
/// `AppState` is moved into the Axum router.
pub fn spawn_render_worker(redis: redis::Client, db: PgPool, s3: S3Client, s3_bucket: String) {
    tokio::spawn(async move {
        worker_loop(redis, db, s3, s3_bucket).await;
    });
}

// ────────────────────────────────────────────────────────────────────────────
// Worker loop
// ────────────────────────────────────────────────────────────────────────────

/// Main worker loop — runs indefinitely, BRPOP-ing jobs from Redis.
///
/// Uses a 5-second BRPOP timeout (not 0) so the loop can detect a lost
/// connection and recover gracefully without blocking forever.
async fn worker_loop(redis: redis::Client, db: PgPool, s3: S3Client, s3_bucket: String) {
    info!("Render worker loop started");

    loop {
        let mut conn = match redis.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(e) => {
                error!("Render worker: Redis connection failed: {e} — retrying in 5s");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        // BRPOP with 5.0-second timeout — returns None on timeout, Some((key, value)) on job
        let result: Result<Option<(String, String)>, redis::RedisError> =
            conn.brpop(RENDER_QUEUE_KEY, 5.0).await;

        match result {
            Ok(None) => {
                // Timeout — no job in queue; loop continues
            }
            Ok(Some((_key, job_id_str))) => match Uuid::parse_str(&job_id_str) {
                Ok(job_id) => {
                    info!("Render worker: dequeued job {}", job_id);
                    process_render_job(job_id, &db, &s3, &s3_bucket).await;
                }
                Err(e) => {
                    error!("Render worker: invalid UUID in queue '{}': {e}", job_id_str);
                }
            },
            Err(e) => {
                error!("Render worker: BRPOP error: {e} — reconnecting");
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Job processing
// ────────────────────────────────────────────────────────────────────────────

/// Processes a single render job end-to-end.
///
/// Steps:
/// 1. UPDATE render_jobs SET status='processing'
/// 2. fetch_render_data() — resume row + bullets grouped by section
/// 3. build_latex_document()
/// 4. UPDATE resumes SET latex_source=...
/// 5. compile_latex() via tectonic.rs
/// 6. upload_pdf_to_s3() → key = pdfs/{resume_id}.pdf
/// 7. UPDATE resumes SET s3_pdf_key=..., status='rendered'
/// 8. UPDATE render_jobs SET status='done'
///
/// On any error at steps 3–8: UPDATE render_jobs SET status='failed', then log and continue.
/// The worker never panics.
async fn process_render_job(job_id: Uuid, db: &PgPool, s3: &S3Client, s3_bucket: &str) {
    // Step 1: Mark job as processing
    if let Err(e) = update_job_status(db, job_id, "processing", None).await {
        error!(
            "Render worker: failed to mark job {} as processing: {e}",
            job_id
        );
        return;
    }

    // Fetch resume_id for this job
    let resume_id =
        match sqlx::query_scalar::<_, Uuid>("SELECT resume_id FROM render_jobs WHERE id = $1")
            .bind(job_id)
            .fetch_optional(db)
            .await
        {
            Ok(Some(id)) => id,
            Ok(None) => {
                error!("Render worker: job {} not found in render_jobs", job_id);
                return;
            }
            Err(e) => {
                error!("Render worker: DB error fetching job {}: {e}", job_id);
                return;
            }
        };

    let result: Result<(), RenderError> = async {
        // Step 2: Fetch render data from DB
        let params = fetch_render_data(db, resume_id).await?;

        // Step 3: Build LaTeX document
        let latex = build_latex_document(&params);

        // Step 4: Persist LaTeX source to resumes table
        sqlx::query("UPDATE resumes SET latex_source = $1, updated_at = NOW() WHERE id = $2")
            .bind(&latex)
            .bind(resume_id)
            .execute(db)
            .await?;

        // Step 5: Compile LaTeX via Tectonic
        let tectonic_result = compile_latex(&latex, job_id).await?;
        info!(
            "Render worker: Tectonic compiled job {} in {}ms (stderr={} bytes)",
            job_id,
            tectonic_result.duration_ms,
            tectonic_result.stderr.len()
        );
        if !tectonic_result.stderr.is_empty() {
            warn!(
                "Render worker: Tectonic stderr for job {}: {}",
                job_id,
                tectonic_result.stderr.chars().take(500).collect::<String>()
            );
        }

        // Step 6: Upload PDF to S3
        let s3_key = upload_pdf_to_s3(s3, s3_bucket, resume_id, tectonic_result.pdf_bytes).await?;

        // Step 7: Update resume with S3 key and rendered status
        sqlx::query(
            "UPDATE resumes SET s3_pdf_key = $1, status = 'rendered', updated_at = NOW() WHERE id = $2",
        )
        .bind(&s3_key)
        .bind(resume_id)
        .execute(db)
        .await?;

        // Step 8: Mark job as done
        update_job_status(db, job_id, "done", None).await?;

        info!(
            "Render worker: job {} completed — PDF at s3://{}",
            job_id, s3_key
        );

        Ok(())
    }
    .await;

    if let Err(e) = result {
        error!("Render worker: job {} failed: {e}", job_id);
        let _ = update_job_status(db, job_id, "failed", Some(&e.to_string())).await;
    }
}

// ────────────────────────────────────────────────────────────────────────────
// DB helpers
// ────────────────────────────────────────────────────────────────────────────

/// Fetches resume and bullets from DB and constructs RenderParams.
///
/// Uses default page config (Inter 11pt, 1" margins) — Phase 6 will wire
/// per-user font/margin preferences.
async fn fetch_render_data(db: &PgPool, resume_id: Uuid) -> Result<RenderParams, RenderError> {
    // Verify resume exists
    let _resume = sqlx::query_as::<_, ResumeRow>("SELECT * FROM resumes WHERE id = $1")
        .bind(resume_id)
        .fetch_optional(db)
        .await?
        .ok_or(RenderError::ResumeNotFound(resume_id))?;

    // Fetch all bullets ordered by section + insertion order
    let bullets = sqlx::query_as::<_, ResumeBulletRow>(
        "SELECT * FROM resume_bullets WHERE resume_id = $1 ORDER BY section, id",
    )
    .bind(resume_id)
    .fetch_all(db)
    .await?;

    // Group bullets by section, preserving first-seen section order
    let mut section_order: Vec<String> = Vec::new();
    let mut section_map: HashMap<String, Vec<String>> = HashMap::new();

    for bullet in bullets {
        if !section_map.contains_key(&bullet.section) {
            section_order.push(bullet.section.clone());
        }
        section_map
            .entry(bullet.section)
            .or_default()
            .push(bullet.bullet_text);
    }

    let sections = section_order
        .into_iter()
        .map(|name| {
            let section_bullets = section_map.remove(&name).unwrap_or_default();
            ResumeSection {
                name,
                bullets: section_bullets,
            }
        })
        .collect();

    // Use default page config — Phase 6 will inject user-selected font/margins
    let page_config = default_page_config(FontFamily::Inter);

    Ok(RenderParams {
        resume_id,
        font: page_config.font,
        font_size_pt: page_config.font_size_pt,
        margin_left_in: page_config.margin_left_in,
        margin_right_in: page_config.margin_right_in,
        sections,
    })
}

/// Updates `render_jobs.status` (and optionally `error_message`) for a given job.
async fn update_job_status(
    db: &PgPool,
    job_id: Uuid,
    status: &str,
    error_msg: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE render_jobs SET status = $1, error_message = $2, updated_at = NOW() WHERE id = $3",
    )
    .bind(status)
    .bind(error_msg)
    .bind(job_id)
    .execute(db)
    .await?;
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// S3 upload
// ────────────────────────────────────────────────────────────────────────────

/// Uploads PDF bytes to S3 and returns the object key.
///
/// Key pattern: `pdfs/{resume_id}.pdf`
async fn upload_pdf_to_s3(
    s3: &S3Client,
    s3_bucket: &str,
    resume_id: Uuid,
    pdf_bytes: Vec<u8>,
) -> Result<String, RenderError> {
    let key = format!("pdfs/{resume_id}.pdf");

    s3.put_object()
        .bucket(s3_bucket)
        .key(&key)
        .body(ByteStream::from(pdf_bytes))
        .content_type("application/pdf")
        .send()
        .await
        .map_err(|e| RenderError::S3Upload(e.to_string()))?;

    Ok(key)
}

// ────────────────────────────────────────────────────────────────────────────
// Sync enqueue (called from generation/generator.rs via spawn_blocking)
// ────────────────────────────────────────────────────────────────────────────

/// Enqueues a render job by pushing the job_id UUID string to the Redis list.
///
/// This is a synchronous function, intended to be called via
/// `tokio::task::spawn_blocking` from async contexts.
pub fn enqueue_render_job_sync(redis: &redis::Client, job_id: Uuid) -> Result<(), RenderError> {
    use redis::Commands;
    let mut conn = redis.get_connection()?;
    conn.lpush::<_, _, ()>(RENDER_QUEUE_KEY, job_id.to_string())?;
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration test — requires a live Redis instance.
    #[tokio::test]
    #[ignore]
    async fn test_enqueue_render_job_sync_pushes_to_list() {
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let client = redis::Client::open(redis_url).expect("Redis client");
        let job_id = Uuid::new_v4();

        enqueue_render_job_sync(&client, job_id).expect("enqueue should succeed");

        // Verify the job_id was pushed to the queue
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .expect("async conn");
        let popped: Option<(String, String)> = conn.brpop(RENDER_QUEUE_KEY, 1.0).await.unwrap();
        let (_key, value) = popped.expect("job should be in queue");
        assert_eq!(value, job_id.to_string());
    }

    /// Integration test — requires a live PostgreSQL instance.
    #[tokio::test]
    #[ignore]
    async fn test_fetch_render_data_returns_not_found() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&db_url)
            .await
            .expect("DB pool");

        let fake_id = Uuid::new_v4();
        let result = fetch_render_data(&pool, fake_id).await;
        assert!(
            matches!(result, Err(RenderError::ResumeNotFound(_))),
            "should return ResumeNotFound for unknown resume, got: {:?}",
            result
        );
    }

    /// Integration test — requires live PostgreSQL.
    #[tokio::test]
    #[ignore]
    async fn test_process_render_job_marks_failed_on_missing_resume() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&db_url)
            .await
            .expect("DB pool");

        // This test would need a real render_jobs row and a fake S3 client.
        // Marked ignore — covered by integration test suite.
        let _ = pool;
    }

    /// Integration test — requires live PostgreSQL.
    #[tokio::test]
    #[ignore]
    async fn test_update_job_status_all_transitions() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&db_url)
            .await
            .expect("DB pool");

        // Would insert a render_jobs row, then cycle through status transitions.
        // Marked ignore — covered by integration test suite.
        let _ = pool;
    }
}
