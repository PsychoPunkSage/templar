//! Background context ingest worker: Redis BRPOP → DB fetch → LLM parse → confirm → DB update.
//!
//! Queue pattern: LPUSH on enqueue, BRPOP with 5-second timeout on dequeue.
//! Redis key stores `item_id` as a UUID string; worker fetches all data from DB.
//!
//! The worker NEVER panics on individual item failure — it logs the error and
//! continues processing the next item.
//!
//! Multiple worker instances can be spawned (INGEST_WORKER_COUNT env var, default 4).
//! Each worker independently drains the same Redis queue — zero coordination needed.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use aws_sdk_s3::Client as S3Client;
use sqlx::PgPool;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::context::batch;
use crate::context::ingest::{confirm_ingest, parse_and_validate, IngestConfirmRequest};
use crate::llm_client::LlmClient;

/// Redis list key for the context ingest job queue.
pub const INGEST_QUEUE_KEY: &str = "context:ingest:queue";

/// Run cleanup every N completed jobs to delete expired batches.
const CLEANUP_EVERY_N_JOBS: u64 = 100;

// ────────────────────────────────────────────────────────────────────────────
// Worker spawn
// ────────────────────────────────────────────────────────────────────────────

/// Spawns a context ingest background worker as a detached tokio task.
///
/// The worker processes ingest items from the Redis queue indefinitely.
/// Call this N times in main.rs to create N parallel workers.
pub fn spawn_context_ingest_worker(
    redis: redis::Client,
    db: PgPool,
    llm: LlmClient,
    s3: S3Client,
    s3_bucket: String,
) {
    tokio::spawn(async move {
        worker_loop(redis, db, llm, s3, s3_bucket).await;
    });
}

// ────────────────────────────────────────────────────────────────────────────
// Worker loop
// ────────────────────────────────────────────────────────────────────────────

/// Main worker loop — runs indefinitely, BRPOP-ing item IDs from Redis.
///
/// Uses a 5-second BRPOP timeout (not 0) so the loop can detect a lost
/// connection and recover gracefully without blocking forever.
async fn worker_loop(
    redis: redis::Client,
    db: PgPool,
    llm: LlmClient,
    s3: S3Client,
    s3_bucket: String,
) {
    info!("Context ingest worker loop started");

    let jobs_completed = Arc::new(AtomicU64::new(0));

    loop {
        let mut conn = match redis.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(e) => {
                error!(
                    "Ingest worker: Redis connection failed: {e} — retrying in 5s"
                );
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }
        };

        // BRPOP with 5.0-second timeout — returns None on timeout, Some on item
        let result: Result<Option<(String, String)>, redis::RedisError> = redis::cmd("BRPOP")
            .arg(INGEST_QUEUE_KEY)
            .arg(5.0_f64)
            .query_async(&mut conn)
            .await;

        match result {
            Ok(None) => {
                // Timeout — no item in queue; loop continues
            }
            Ok(Some((_key, item_id_str))) => {
                let item_id_str = item_id_str.trim().to_string();
                match Uuid::parse_str(&item_id_str) {
                    Ok(item_id) => {
                        info!(%item_id, "Ingest worker: dequeued item");
                        if let Err(e) =
                            process_ingest_item(item_id, &db, &llm, &s3, &s3_bucket).await
                        {
                            error!(%item_id, error = %e, "Ingest worker: item processing failed");
                            // Best-effort mark failed — if this also errors, just log it
                            if let Err(db_err) =
                                batch::mark_item_failed(&db, item_id, &e.to_string()).await
                            {
                                error!(%item_id, db_error = %db_err, "Ingest worker: failed to update item status after error");
                            }
                        }

                        // Periodic cleanup of expired batches
                        let count = jobs_completed.fetch_add(1, Ordering::Relaxed) + 1;
                        if count % CLEANUP_EVERY_N_JOBS == 0 {
                            match batch::cleanup_expired_batches(&db).await {
                                Ok(n) if n > 0 => {
                                    info!("Ingest worker: cleaned up {n} expired batches")
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    warn!("Ingest worker: batch cleanup error: {e}")
                                }
                            }
                        }
                    }
                    Err(e) => {
                        error!("Ingest worker: invalid UUID in queue '{}': {e}", item_id_str);
                    }
                }
            }
            Err(e) => {
                error!("Ingest worker: BRPOP error: {e} — reconnecting");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Item processing
// ────────────────────────────────────────────────────────────────────────────

/// Processes a single ingest item end-to-end.
///
/// Steps:
/// 1. Fetch item text and user_id from DB
/// 2. Mark item as 'processing'
/// 3. `parse_and_validate` — LLM parse + impact validation
/// 4. Check impact_validation.passed — fail fast if not passed
/// 5. `confirm_ingest` — commit to context_entries + S3 snapshot
/// 6. Mark item as succeeded with the resulting entry_id
///
/// On any error: marks item as failed and returns Ok(()) — the worker continues.
/// The caller's outer error handler provides a safety net for unexpected panics.
async fn process_ingest_item(
    item_id: Uuid,
    db: &PgPool,
    llm: &LlmClient,
    s3: &S3Client,
    s3_bucket: &str,
) -> anyhow::Result<()> {
    // Step 1: Fetch item from DB
    let Some((_batch_id, user_id, entry_text)) = batch::get_item(db, item_id).await? else {
        warn!(%item_id, "Ingest worker: item not found in DB, skipping");
        return Ok(());
    };

    // Step 2: Mark item as processing
    batch::mark_item_processing(db, item_id).await?;

    // Step 3: Parse and validate via LLM
    let preview = match parse_and_validate(&entry_text, llm, db, user_id).await {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("Parse failed: {e}");
            error!(%item_id, %user_id, error = %e, "Ingest worker: parse_and_validate failed");
            batch::mark_item_failed(db, item_id, &msg).await?;
            return Ok(());
        }
    };

    // Step 4: Check impact validation
    if !preview.impact_validation.passed {
        let missing: Vec<String> = preview
            .impact_validation
            .missing
            .iter()
            .map(|gap| gap.reason.clone())
            .collect();
        let missing_str = missing.join("; ");
        let msg = format!(
            "Impact validation failed — quantification required: {missing_str}. \
             Please add specific metrics (e.g., percentages, dollar amounts, user counts)."
        );
        warn!(%item_id, %user_id, "Ingest worker: impact validation failed");
        batch::mark_item_failed(db, item_id, &msg).await?;
        return Ok(());
    }

    // Step 5: Confirm ingest (commit to context_entries + S3)
    let confirm_req = IngestConfirmRequest {
        user_id,
        entry: preview.entry,
        acknowledged_gaps: vec![],
    };
    let confirm_resp = match confirm_ingest(db, s3, s3_bucket, &confirm_req).await {
        Ok(r) => r,
        Err(e) => {
            let msg = format!("Commit failed: {e}");
            error!(%item_id, %user_id, error = %e, "Ingest worker: confirm_ingest failed");
            batch::mark_item_failed(db, item_id, &msg).await?;
            return Ok(());
        }
    };

    // Step 6: Mark item as succeeded
    batch::mark_item_succeeded(db, item_id, confirm_resp.entry_id).await?;
    info!(
        %item_id,
        %user_id,
        entry_id = %confirm_resp.entry_id,
        completeness_delta = confirm_resp.completeness_delta,
        "Ingest worker: item succeeded"
    );

    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    /// Integration tests require live Redis + PostgreSQL.
    /// Unit-testable logic is covered by splitter.rs and batch.rs tests.

    /// Integration test — requires live Redis + PostgreSQL.
    #[tokio::test]
    #[ignore]
    async fn test_worker_processes_item_end_to_end() {
        // Would enqueue a real item and verify it transitions to succeeded/failed.
        // Covered by integration test suite.
    }
}
