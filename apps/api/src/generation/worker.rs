//! Background generation worker — decouples resume generation from the HTTP request lifecycle.
//!
//! Queue pattern: LPUSH on enqueue, BRPOP with 5-second timeout on dequeue.
//! Queue key: `generation:jobs`
//!
//! This mirrors the render worker pattern in `render/worker.rs` exactly.
//!
//! Lifecycle:
//!   1. `handle_generate` (HTTP handler) inserts a `generation_jobs` row and enqueues job_id.
//!   2. This worker dequeues the job_id, deserializes the stored `GenerateRequest`, and
//!      calls `generate_resume()` — the core pipeline is unchanged.
//!   3. On success: UPDATE generation_jobs SET status='done', result=<full GenerateResponse JSON>.
//!   4. On failure: UPDATE generation_jobs SET status='failed', error=<message>.
//!   5. The frontend polls GET /api/v1/generation/jobs/:id/status every 3 seconds.
//!
//! A job is NEVER left at 'processing'. All error paths mark the job 'failed'.

use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::Semaphore;
use tokio::time::Duration;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::config::Config;
use crate::errors::AppError;
use crate::generation::fit_scoring::FitScorer;
use crate::generation::generator::{generate_resume, GenerateRequest, GenerateResponse};
use crate::layout::PageConfig;
use crate::llm_client::LlmClient;

pub const GENERATION_QUEUE_KEY: &str = "generation:jobs";

// ────────────────────────────────────────────────────────────────────────────
// Public spawn entry point
// ────────────────────────────────────────────────────────────────────────────

/// Spawns the generation background worker as a detached Tokio task.
///
/// This function returns immediately — the worker runs indefinitely in the background.
/// Call once from `main.rs` after building AppState.
pub fn spawn_generation_worker(
    redis: redis::Client,
    db: PgPool,
    llm: LlmClient,
    fit_scorer: Arc<dyn FitScorer>,
    page_config: PageConfig,
    config: Config,
) {
    let concurrency = config.generation_worker_count;
    tokio::spawn(async move {
        worker_loop(redis, db, llm, fit_scorer, page_config, config, concurrency).await;
    });
}

// ────────────────────────────────────────────────────────────────────────────
// Worker loop
// ────────────────────────────────────────────────────────────────────────────

/// Main worker loop — runs indefinitely, BRPOP-ing jobs from Redis.
///
/// Concurrency model: up to `generation_worker_count` jobs run in parallel.
/// A semaphore provides back-pressure: the BRPOP loop blocks when all slots are
/// occupied so Redis doesn't accumulate jobs faster than we can process them.
///
/// Note: generation_worker_count should be kept low (default 2) because each
/// generate_resume() call already saturates LLM concurrency internally via
/// generation_llm_concurrency and grounding_llm_concurrency semaphores.
async fn worker_loop(
    redis: redis::Client,
    db: PgPool,
    llm: LlmClient,
    fit_scorer: Arc<dyn FitScorer>,
    page_config: PageConfig,
    config: Config,
    concurrency: usize,
) {
    info!("Generation worker loop started (concurrency: {concurrency})");

    let semaphore = Arc::new(Semaphore::new(concurrency));

    loop {
        let mut conn = match redis.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(e) => {
                error!("Generation worker: Redis connection failed: {e} — retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        // Block here if all slots are in use — prevents dequeuing more work than
        // we can handle. acquire_owned() gives an OwnedSemaphorePermit that can
        // be moved into the spawned task and released when the task completes.
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .expect("semaphore closed — this never happens");

        // BRPOP with 5.0-second timeout
        let result: Result<Option<(String, String)>, redis::RedisError> =
            redis::AsyncCommands::brpop(&mut conn, GENERATION_QUEUE_KEY, 5.0).await;

        match result {
            Ok(None) => {
                // Timeout — no job arrived. Drop permit so we don't hold a slot while idle.
                drop(permit);
            }
            Ok(Some((_key, job_id_str))) => match Uuid::parse_str(&job_id_str) {
                Ok(job_id) => {
                    info!("Generation worker: dequeued job {}", job_id);

                    let (db2, llm2, fs2, pc2, cfg2, redis2) = (
                        db.clone(),
                        llm.clone(),
                        Arc::clone(&fit_scorer),
                        page_config.clone(),
                        config.clone(),
                        redis.clone(),
                    );

                    tokio::spawn(async move {
                        let _permit = permit; // holds the slot for this job's lifetime
                        process_generation_job(job_id, &db2, &llm2, fs2, &pc2, &cfg2, &redis2).await;
                    });
                }
                Err(e) => {
                    drop(permit);
                    error!("Generation worker: invalid UUID in queue '{}': {e}", job_id_str);
                }
            },
            Err(e) => {
                drop(permit);
                error!("Generation worker: BRPOP error: {e} — reconnecting");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Job processing
// ────────────────────────────────────────────────────────────────────────────

/// Processes a single generation job end-to-end.
///
/// Steps:
/// 1. Mark job 'processing'
/// 2. Fetch and deserialize GenerateRequest from generation_jobs.request
/// 3. Call generate_resume() — full pipeline unchanged
/// 4. Store full GenerateResponse as result JSONB
/// 5. Mark job 'done' with resume_id
///
/// All errors mark the job 'failed'. A job is NEVER left at 'processing'.
async fn process_generation_job(
    job_id: Uuid,
    db: &PgPool,
    llm: &LlmClient,
    fit_scorer: Arc<dyn FitScorer>,
    page_config: &PageConfig,
    config: &Config,
    redis: &redis::Client,
) {
    info!(job_id = %job_id, "Generation job dequeued — starting processing");

    // Step 1: Mark as processing
    if let Err(e) = update_job_status(db, job_id, "processing", None, None).await {
        error!(job_id = %job_id, error = %e,
            "Generation worker: failed to mark job as processing — continuing anyway");
    }

    // Step 2: Fetch the stored GenerateRequest from the DB row
    let request_value: serde_json::Value = match sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT request FROM generation_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_optional(db)
    .await
    {
        Ok(Some(v)) => v,
        Ok(None) => {
            error!(job_id = %job_id, "Generation job row not found — marking failed");
            let _ = update_job_status(db, job_id, "failed", None,
                Some("Generation job row not found in database")).await;
            return;
        }
        Err(e) => {
            error!(job_id = %job_id, error = %e, "DB error fetching generation job — marking failed");
            let _ = update_job_status(db, job_id, "failed", None,
                Some(&format!("DB error fetching job: {e}"))).await;
            return;
        }
    };

    // Step 3: Deserialize the stored request payload
    let request: GenerateRequest = match serde_json::from_value(request_value) {
        Ok(r) => r,
        Err(e) => {
            error!(job_id = %job_id, error = %e, "Failed to deserialize GenerateRequest — marking failed");
            let _ = update_job_status(db, job_id, "failed", None,
                Some(&format!("Failed to deserialize request: {e}"))).await;
            return;
        }
    };

    // Step 4: Run the full generation pipeline (unchanged from HTTP handler path)
    let result: Result<GenerateResponse, AppError> = generate_resume(
        db,
        llm,
        fit_scorer.as_ref(),
        page_config,
        Some(redis),
        true, // grounding_enabled: always true in production
        request,
        config,
    )
    .await;

    // Step 5: Persist result or failure
    match result {
        Ok(response) => {
            let resume_id = response.resume_id;

            let result_json = match serde_json::to_value(&response) {
                Ok(v) => v,
                Err(e) => {
                    error!(job_id = %job_id, resume_id = %resume_id, error = %e,
                        "Failed to serialize GenerateResponse — marking failed");
                    let _ = update_job_status(db, job_id, "failed", None,
                        Some(&format!("Failed to serialize result: {e}"))).await;
                    return;
                }
            };

            if let Err(e) = update_job_status(db, job_id, "done", Some(resume_id), None).await {
                error!(job_id = %job_id, error = %e, "Failed to mark generation job done");
            }
            // Store result JSONB separately (update_job_status handles status+resume_id)
            if let Err(e) = sqlx::query(
                "UPDATE generation_jobs SET result = $1 WHERE id = $2"
            )
            .bind(result_json)
            .bind(job_id)
            .execute(db)
            .await
            {
                // Non-fatal: job is already marked done. The status endpoint will
                // return done with null entry_groups, which the frontend handles gracefully.
                warn!(job_id = %job_id, resume_id = %resume_id, error = %e,
                    "Failed to store generation result JSONB — job still marked done");
            }

            info!(job_id = %job_id, resume_id = %resume_id, "Generation job completed successfully");
        }
        Err(e) => {
            error!(job_id = %job_id, error = %e, "Generation pipeline failed — marking job failed");
            let _ = update_job_status(db, job_id, "failed", None,
                Some(&format!("Generation failed: {e}"))).await;
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// DB status helper
// ────────────────────────────────────────────────────────────────────────────

/// Updates `generation_jobs.status` and optionally `resume_id` / `error`.
async fn update_job_status(
    db: &PgPool,
    job_id: Uuid,
    status: &str,
    resume_id: Option<Uuid>,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE generation_jobs
         SET status = $1, resume_id = $2, error = $3, updated_at = NOW()
         WHERE id = $4",
    )
    .bind(status)
    .bind(resume_id)
    .bind(error)
    .bind(job_id)
    .execute(db)
    .await?;
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Sync enqueue (called via spawn_blocking from the HTTP handler)
// ────────────────────────────────────────────────────────────────────────────

/// Enqueues a generation job by LPUSH-ing the job_id UUID string to the Redis list.
///
/// Synchronous — intended to be called via `tokio::task::spawn_blocking`.
pub fn enqueue_generation_job(redis: &redis::Client, job_id: Uuid) -> Result<(), anyhow::Error> {
    use redis::Commands;
    let mut conn = redis.get_connection()?;
    conn.lpush::<_, _, ()>(GENERATION_QUEUE_KEY, job_id.to_string())?;
    Ok(())
}
