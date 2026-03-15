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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
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
                error!("Ingest worker: Redis connection failed: {e} — retrying in 5s");
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
                        if count.is_multiple_of(CLEANUP_EVERY_N_JOBS) {
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
                        error!(
                            "Ingest worker: invalid UUID in queue '{}': {e}",
                            item_id_str
                        );
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
/// 3. `parse_and_validate` — LLM parse (Phase 5.5: quality is non-blocking)
/// 4. Check for duplicate/merge via `dedup::check_and_merge`
///    5a. Duplicate found → `merger::merge_and_commit` → mark merged
///    5b. No duplicate  → `confirm_ingest` → commit to context_entries + S3 → mark succeeded
///
/// On any error: marks item as failed and returns Ok(()) — the worker continues.
async fn process_ingest_item(
    item_id: Uuid,
    db: &PgPool,
    llm: &LlmClient,
    s3: &S3Client,
    s3_bucket: &str,
) -> anyhow::Result<()> {
    use crate::context::dedup::DedupResult;
    use crate::context::merger;
    use crate::context::versioning::get_current_entries;

    // Step 1: Fetch item from DB
    let Some((_batch_id, user_id, entry_text)) = batch::get_item(db, item_id).await? else {
        warn!(%item_id, "Ingest worker: item not found in DB, skipping");
        return Ok(());
    };

    // Step 2: Mark item as processing
    batch::mark_item_processing(db, item_id).await?;

    // Step 3: Parse and validate via LLM (quality is non-blocking — always proceeds)
    let preview = match parse_and_validate(&entry_text, llm, db, user_id).await {
        Ok(p) => p,
        Err(e) => {
            let msg = format!("Parse failed: {e}");
            error!(%item_id, %user_id, error = %e, "Ingest worker: parse_and_validate failed");
            batch::mark_item_failed(db, item_id, &msg).await?;
            return Ok(());
        }
    };

    if preview.quality.quality_score < 1.0 {
        info!(
            %item_id,
            %user_id,
            quality_score = preview.quality.quality_score,
            flags = ?preview.quality.flags,
            "Ingest worker: low quality entry — storing with hints (non-blocking)"
        );
    }

    // Step 4: Check for duplicate (heuristic + LLM confirm)
    let existing_entries = match get_current_entries(db, user_id).await {
        Ok(e) => e,
        Err(e) => {
            warn!(%item_id, %user_id, error = %e, "Ingest worker: dedup lookup failed, proceeding without dedup");
            vec![]
        }
    };

    let dedup_result =
        crate::context::dedup::check_and_merge(&existing_entries, &preview.entry, llm).await;

    match dedup_result {
        DedupResult::Merged(existing_entry_id) => {
            // Step 5a: Merge new text into existing entry (LLM merge + new version)
            info!(%item_id, %user_id, %existing_entry_id, "Ingest worker: merging with existing entry");
            match merger::merge_and_commit(
                db,
                s3,
                s3_bucket,
                llm,
                user_id,
                existing_entry_id,
                &preview.entry,
            )
            .await
            {
                Ok(()) => {
                    batch::mark_item_merged(db, item_id, existing_entry_id).await?;
                    info!(%item_id, %user_id, %existing_entry_id, "Ingest worker: merge succeeded");
                }
                Err(e) => {
                    // Merge failed — fall back to normal insert
                    warn!(%item_id, %user_id, error = %e, "Ingest worker: merge failed, falling back to normal insert");
                    let confirm_req = IngestConfirmRequest {
                        user_id,
                        entry: preview.entry,
                        acknowledged_gaps: vec![],
                    };
                    commit_and_mark(db, s3, s3_bucket, item_id, user_id, confirm_req).await?;
                }
            }
        }
        DedupResult::New | DedupResult::Error(_) => {
            // Step 5b: Normal insert
            if let DedupResult::Error(ref e) = dedup_result {
                warn!(%item_id, %user_id, error = %e, "Ingest worker: dedup check failed, proceeding with insert");
            }
            let confirm_req = IngestConfirmRequest {
                user_id,
                entry: preview.entry,
                acknowledged_gaps: vec![],
            };
            commit_and_mark(db, s3, s3_bucket, item_id, user_id, confirm_req).await?;
        }
    }

    Ok(())
}

/// Helper: confirm_ingest + mark_item_succeeded.
async fn commit_and_mark(
    db: &PgPool,
    s3: &S3Client,
    s3_bucket: &str,
    item_id: Uuid,
    user_id: Uuid,
    confirm_req: IngestConfirmRequest,
) -> anyhow::Result<()> {
    match confirm_ingest(db, s3, s3_bucket, &confirm_req).await {
        Ok(r) => {
            batch::mark_item_succeeded(db, item_id, r.entry_id).await?;
            info!(
                %item_id,
                %user_id,
                entry_id = %r.entry_id,
                completeness_delta = r.completeness_delta,
                "Ingest worker: item succeeded"
            );
            Ok(())
        }
        Err(e) => {
            let msg = format!("Commit failed: {e}");
            error!(%item_id, %user_id, error = %e, "Ingest worker: confirm_ingest failed");
            batch::mark_item_failed(db, item_id, &msg).await?;
            Ok(())
        }
    }
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
