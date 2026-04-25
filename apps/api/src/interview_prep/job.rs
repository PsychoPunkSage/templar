//! Interview prep background worker.
//!
//! Queue pattern: LPUSH on enqueue, BRPOP with 5-second timeout on dequeue.
//! Queue key: `interview_prep:jobs`
//!
//! Worker count: INTERVIEW_PREP_WORKER_COUNT env var (default 2).
//!
//! Lifecycle:
//!   1. `enqueue_prep_job` (called from generation/worker.rs auto-trigger OR
//!      POST /trigger handler) LPUSHes project_id to Redis AND upserts meta
//!      status=generating (so the frontend sees "Updating prep..." immediately).
//!   2. Worker dequeues project_id UUID.
//!   3. Calls `generate_prep()` — full pipeline.
//!   4. On success: meta is already updated to 'ready' by generate_prep().
//!   5. On failure: UPDATE meta SET status='failed'.
//!
//! A job is NEVER left at 'generating'. All error paths mark the meta 'failed'.

use sqlx::PgPool;
use tokio::time::Duration;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::interview_prep::generator::generate_prep;

pub const INTERVIEW_PREP_QUEUE_KEY: &str = "interview_prep:jobs";

// ────────────────────────────────────────────────────────────────────────────
// Enqueue
// ────────────────────────────────────────────────────────────────────────────

/// Enqueue an interview prep job.
///
/// This is an ASYNC function called from both:
///   - `generation/worker.rs` fire-and-forget spawn (auto-trigger after resume generation)
///   - POST /:project_id/trigger HTTP handler
///
/// It:
///   1. Upserts interview_prep_meta with status=generating (so callers see "Updating prep..." immediately).
///   2. LPUSHes the project_id UUID to Redis.
pub async fn enqueue_prep_job(redis: &redis::Client, db: &PgPool, project_id: Uuid) -> Result<(), anyhow::Error> {
    // Step 1: Upsert meta to generating BEFORE enqueuing
    sqlx::query(
        r#"INSERT INTO interview_prep_meta (project_id, status)
           VALUES ($1, 'generating')
           ON CONFLICT (project_id) DO UPDATE SET
               status     = 'generating',
               updated_at = NOW()"#,
    )
    .bind(project_id)
    .execute(db)
    .await
    .map_err(|e| anyhow::anyhow!("Failed to upsert prep meta: {e}"))?;

    // Step 2: Enqueue to Redis
    let mut conn = redis.get_multiplexed_async_connection().await?;
    redis::AsyncCommands::lpush::<_, _, ()>(&mut conn, INTERVIEW_PREP_QUEUE_KEY, project_id.to_string()).await?;

    info!(project_id = %project_id, "Interview prep job enqueued");
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Public spawn entry point
// ────────────────────────────────────────────────────────────────────────────

/// Spawns N interview prep background workers as detached Tokio tasks.
///
/// Returns immediately — workers run indefinitely.
pub fn spawn_interview_prep_worker(redis: redis::Client, db: PgPool, llm: crate::llm_client::LlmClient, count: usize) {
    for i in 0..count {
        let redis2 = redis.clone();
        let db2 = db.clone();
        let llm2 = llm.clone();
        tokio::spawn(async move {
            info!("Interview prep worker {} started", i);
            worker_loop(redis2, db2, llm2).await;
        });
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Worker loop
// ────────────────────────────────────────────────────────────────────────────

async fn worker_loop(redis: redis::Client, db: PgPool, llm: crate::llm_client::LlmClient) {
    loop {
        let mut conn = match redis.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(e) => {
                error!("Interview prep worker: Redis connection failed: {e} — retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        let result: Result<Option<(String, String)>, redis::RedisError> =
            redis::AsyncCommands::brpop(&mut conn, INTERVIEW_PREP_QUEUE_KEY, 5.0).await;

        match result {
            Ok(None) => {} // timeout — nothing to do
            Ok(Some((_key, id_str))) => match Uuid::parse_str(&id_str) {
                Ok(project_id) => {
                    info!(project_id = %project_id, "Interview prep worker: dequeued job");
                    let db2 = db.clone();
                    let llm2 = llm.clone();
                    tokio::spawn(async move {
                        process_prep_job(project_id, &db2, &llm2).await;
                    });
                }
                Err(e) => {
                    error!("Interview prep worker: invalid UUID '{}': {e}", id_str);
                }
            },
            Err(e) => {
                error!("Interview prep worker: BRPOP error: {e} — reconnecting");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Job processing
// ────────────────────────────────────────────────────────────────────────────

async fn process_prep_job(project_id: Uuid, db: &PgPool, llm: &crate::llm_client::LlmClient) {
    info!(project_id = %project_id, "Interview prep job starting");

    match generate_prep(db, llm, project_id).await {
        Ok(()) => {
            info!(project_id = %project_id, "Interview prep job completed successfully");
        }
        Err(e) => {
            error!(project_id = %project_id, error = %e, "Interview prep generation failed — marking failed");
            if let Err(db_err) = sqlx::query(
                "UPDATE interview_prep_meta SET status = 'failed', updated_at = NOW() WHERE project_id = $1",
            )
            .bind(project_id)
            .execute(db)
            .await
            {
                warn!(project_id = %project_id, error = %db_err, "Failed to mark prep meta as failed");
            }
        }
    }
}
