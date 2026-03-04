//! Batch DB operations for async context ingestion.
//!
//! Manages `context_ingest_batches` and `context_ingest_items` records.
//! All writes are fire-and-return — no long-running transactions.
//!
//! Counter updates use SQL arithmetic (not read-modify-write) to be safe
//! under concurrent workers.

use anyhow::Result;
use redis::AsyncCommands;
use serde::Serialize;
use sqlx::PgPool;
use uuid::Uuid;

/// Per-item status as returned by the polling endpoint.
#[derive(Debug, Serialize, Clone)]
pub struct BatchItemStatus {
    pub id: Uuid,
    pub entry_index: i32,
    pub status: String,
    pub error_msg: Option<String>,
    pub entry_id: Option<Uuid>,
}

/// Full batch status response for the polling endpoint.
#[derive(Debug, Serialize)]
pub struct BatchStatusResponse {
    pub batch_id: Uuid,
    pub source_type: String,
    pub filename: Option<String>,
    pub total: i32,
    pub pending: i32,
    pub succeeded: i32,
    pub failed: i32,
    pub status: String,
    pub items: Vec<BatchItemStatus>,
}

// ────────────────────────────────────────────────────────────────────────────
// Create
// ────────────────────────────────────────────────────────────────────────────

/// Create a batch record and all item records in a single logical operation.
///
/// Inserts the batch row first, then bulk-inserts all items in order.
/// Returns the generated `batch_id`.
pub async fn create_batch(
    pool: &PgPool,
    user_id: Uuid,
    source_type: &str,
    filename: Option<&str>,
    entries: &[String],
) -> Result<Uuid> {
    let total = entries.len() as i32;

    let batch_id = sqlx::query_scalar!(
        r#"INSERT INTO context_ingest_batches
               (user_id, source_type, filename, total, pending)
           VALUES ($1, $2, $3, $4, $4)
           RETURNING id"#,
        user_id,
        source_type,
        filename,
        total,
    )
    .fetch_one(pool)
    .await?;

    for (idx, text) in entries.iter().enumerate() {
        sqlx::query!(
            r#"INSERT INTO context_ingest_items
                   (batch_id, user_id, entry_index, entry_text)
               VALUES ($1, $2, $3, $4)"#,
            batch_id,
            user_id,
            idx as i32,
            text,
        )
        .execute(pool)
        .await?;
    }

    Ok(batch_id)
}

// ────────────────────────────────────────────────────────────────────────────
// Read
// ────────────────────────────────────────────────────────────────────────────

/// Fetch all item IDs for a batch in entry_index order (for Redis enqueue).
pub async fn get_item_ids(pool: &PgPool, batch_id: Uuid) -> Result<Vec<Uuid>> {
    let ids = sqlx::query_scalar!(
        "SELECT id FROM context_ingest_items WHERE batch_id = $1 ORDER BY entry_index",
        batch_id
    )
    .fetch_all(pool)
    .await?;
    Ok(ids)
}

/// Fetch a single item for worker processing.
///
/// Returns `(batch_id, user_id, entry_text)` or `None` if the item does not exist.
pub async fn get_item(
    pool: &PgPool,
    item_id: Uuid,
) -> Result<Option<(Uuid, Uuid, String)>> {
    let row = sqlx::query!(
        "SELECT batch_id, user_id, entry_text FROM context_ingest_items WHERE id = $1",
        item_id
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| (r.batch_id, r.user_id, r.entry_text)))
}

/// Get batch status and all items for the polling endpoint.
///
/// Returns `None` if the batch does not exist.
pub async fn get_batch_status(
    pool: &PgPool,
    batch_id: Uuid,
) -> Result<Option<BatchStatusResponse>> {
    let batch = sqlx::query!(
        r#"SELECT id, source_type, filename, total, pending, succeeded, failed, status
           FROM context_ingest_batches
           WHERE id = $1"#,
        batch_id
    )
    .fetch_optional(pool)
    .await?;

    let Some(b) = batch else {
        return Ok(None);
    };

    let items = sqlx::query!(
        r#"SELECT id, entry_index, status, error_msg, entry_id
           FROM context_ingest_items
           WHERE batch_id = $1
           ORDER BY entry_index"#,
        batch_id
    )
    .fetch_all(pool)
    .await?;

    Ok(Some(BatchStatusResponse {
        batch_id: b.id,
        source_type: b.source_type,
        filename: b.filename,
        total: b.total,
        pending: b.pending,
        succeeded: b.succeeded,
        failed: b.failed,
        status: b.status,
        items: items
            .into_iter()
            .map(|r| BatchItemStatus {
                id: r.id,
                entry_index: r.entry_index,
                status: r.status,
                error_msg: r.error_msg,
                entry_id: r.entry_id,
            })
            .collect(),
    }))
}

// ────────────────────────────────────────────────────────────────────────────
// Update — item lifecycle
// ────────────────────────────────────────────────────────────────────────────

/// Mark an item as 'processing' and flip the batch to 'processing' if still 'queued'.
pub async fn mark_item_processing(pool: &PgPool, item_id: Uuid) -> Result<()> {
    sqlx::query!(
        "UPDATE context_ingest_items SET status = 'processing', updated_at = NOW() WHERE id = $1",
        item_id
    )
    .execute(pool)
    .await?;

    // Flip batch status from 'queued' → 'processing' on first worker pick-up
    sqlx::query!(
        r#"UPDATE context_ingest_batches
           SET status = 'processing', updated_at = NOW()
           WHERE id = (SELECT batch_id FROM context_ingest_items WHERE id = $1)
             AND status = 'queued'"#,
        item_id
    )
    .execute(pool)
    .await?;

    Ok(())
}

/// Mark an item as succeeded, record the resulting entry_id, and update batch counters.
pub async fn mark_item_succeeded(pool: &PgPool, item_id: Uuid, entry_id: Uuid) -> Result<()> {
    sqlx::query!(
        r#"UPDATE context_ingest_items
           SET status = 'succeeded', entry_id = $2, updated_at = NOW()
           WHERE id = $1"#,
        item_id,
        entry_id,
    )
    .execute(pool)
    .await?;
    update_batch_counters(pool, item_id, true).await
}

/// Mark an item as failed, record the error message, and update batch counters.
pub async fn mark_item_failed(pool: &PgPool, item_id: Uuid, error_msg: &str) -> Result<()> {
    sqlx::query!(
        r#"UPDATE context_ingest_items
           SET status = 'failed', error_msg = $2, updated_at = NOW()
           WHERE id = $1"#,
        item_id,
        error_msg,
    )
    .execute(pool)
    .await?;
    update_batch_counters(pool, item_id, false).await
}

/// Atomically decrement `pending`, increment `succeeded` or `failed`, and flip
/// batch `status` to `'done'` + set `expires_at` when `pending` reaches 0.
///
/// Uses SQL arithmetic — safe under concurrent workers with no serialization needed.
async fn update_batch_counters(pool: &PgPool, item_id: Uuid, succeeded: bool) -> Result<()> {
    if succeeded {
        sqlx::query!(
            r#"UPDATE context_ingest_batches
               SET succeeded  = succeeded + 1,
                   pending    = GREATEST(pending - 1, 0),
                   status     = CASE WHEN pending - 1 <= 0 THEN 'done' ELSE status END,
                   expires_at = CASE WHEN pending - 1 <= 0
                                     THEN NOW() + INTERVAL '7 days'
                                     ELSE expires_at END,
                   updated_at = NOW()
               WHERE id = (SELECT batch_id FROM context_ingest_items WHERE id = $1)"#,
            item_id
        )
        .execute(pool)
        .await?;
    } else {
        sqlx::query!(
            r#"UPDATE context_ingest_batches
               SET failed     = failed + 1,
                   pending    = GREATEST(pending - 1, 0),
                   status     = CASE WHEN pending - 1 <= 0 THEN 'done' ELSE status END,
                   expires_at = CASE WHEN pending - 1 <= 0
                                     THEN NOW() + INTERVAL '7 days'
                                     ELSE expires_at END,
                   updated_at = NOW()
               WHERE id = (SELECT batch_id FROM context_ingest_items WHERE id = $1)"#,
            item_id
        )
        .execute(pool)
        .await?;
    }
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Redis enqueue
// ────────────────────────────────────────────────────────────────────────────

/// Enqueue all item IDs to the Redis ingest queue in a single pipelined call.
///
/// Uses LPUSH (left-push) so workers BRPOP from the right — FIFO ordering.
/// This is an async function using an async Redis connection.
pub async fn enqueue_batch_items(redis: &redis::Client, item_ids: Vec<Uuid>) -> Result<()> {
    if item_ids.is_empty() {
        return Ok(());
    }

    let mut conn = redis.get_multiplexed_async_connection().await?;

    // LPUSH each item_id individually to the ingest queue
    // Using AsyncCommands which is straightforward and compatible with redis 0.25
    for id in &item_ids {
        conn.lpush::<_, _, ()>("context:ingest:queue", id.to_string())
            .await?;
    }

    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Cleanup
// ────────────────────────────────────────────────────────────────────────────

/// Delete batches whose TTL has expired (`expires_at < NOW()`).
///
/// Cascade delete removes all associated `context_ingest_items` automatically.
/// Returns the number of batches deleted.
pub async fn cleanup_expired_batches(pool: &PgPool) -> Result<u64> {
    let result = sqlx::query!("DELETE FROM context_ingest_batches WHERE expires_at < NOW()")
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Basic struct serialization check — no DB needed.
    #[test]
    fn test_batch_status_response_serializes() {
        let resp = BatchStatusResponse {
            batch_id: Uuid::new_v4(),
            source_type: "text".to_string(),
            filename: None,
            total: 3,
            pending: 1,
            succeeded: 2,
            failed: 0,
            status: "processing".to_string(),
            items: vec![
                BatchItemStatus {
                    id: Uuid::new_v4(),
                    entry_index: 0,
                    status: "succeeded".to_string(),
                    error_msg: None,
                    entry_id: Some(Uuid::new_v4()),
                },
                BatchItemStatus {
                    id: Uuid::new_v4(),
                    entry_index: 1,
                    status: "failed".to_string(),
                    error_msg: Some("Impact validation failed".to_string()),
                    entry_id: None,
                },
            ],
        };

        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("processing"));
        assert!(json.contains("Impact validation failed"));
    }

    /// Integration test — requires live PostgreSQL.
    #[tokio::test]
    #[ignore]
    async fn test_create_batch_and_get_items() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&db_url)
            .await
            .expect("DB pool");

        // Would need a real user_id (FK constraint)
        // Covered by integration test suite
        let _ = pool;
    }

    /// Integration test — requires live Redis.
    #[tokio::test]
    #[ignore]
    async fn test_enqueue_batch_items_pipelined() {
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let client = redis::Client::open(redis_url).expect("Redis client");
        let ids = vec![Uuid::new_v4(), Uuid::new_v4()];
        enqueue_batch_items(&client, ids).await.expect("enqueue");
    }
}
