use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;
use crate::generation::fit_scoring::FitReport;

/// Look up a cached FitReport for the given (user_id, jd_hash, context_hash) triple.
///
/// Returns `None` on cache miss. Returns an error only for genuine DB failures —
/// not for JSON deserialisation failures (those are treated as a miss so a fresh
/// score is computed instead of surfacing a confusing internal error).
///
/// Uses runtime query (not `query!` macro) so the table does not need to be present
/// at compile time — the migration runs at startup.
pub async fn lookup_cache(
    pool: &PgPool,
    user_id: Uuid,
    jd_hash: &str,
    context_hash: &str,
) -> Result<Option<FitReport>> {
    let row = sqlx::query_as::<_, (serde_json::Value,)>(
        r#"SELECT fit_report FROM fit_scores
           WHERE user_id = $1 AND jd_hash = $2 AND context_hash = $3
           LIMIT 1"#,
    )
    .bind(user_id)
    .bind(jd_hash)
    .bind(context_hash)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((json_val,)) => {
            match serde_json::from_value::<FitReport>(json_val) {
                Ok(report) => Ok(Some(report)),
                Err(_) => Ok(None), // Deserialisation failure → treat as miss
            }
        }
        None => Ok(None),
    }
}

/// Upsert a FitReport into the cache.
///
/// Uses `ON CONFLICT ... DO UPDATE` so re-scoring the same (user, jd, context)
/// triple refreshes the cached value and resets `created_at`.
pub async fn upsert_cache(
    pool: &PgPool,
    user_id: Uuid,
    jd_hash: &str,
    context_hash: &str,
    fit_report: &FitReport,
) -> Result<()> {
    let report_json = serde_json::to_value(fit_report)?;
    sqlx::query(
        r#"INSERT INTO fit_scores (user_id, jd_hash, context_hash, fit_report)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT ON CONSTRAINT uq_fit_score_cache
           DO UPDATE SET fit_report = EXCLUDED.fit_report, created_at = NOW()"#,
    )
    .bind(user_id)
    .bind(jd_hash)
    .bind(context_hash)
    .bind(report_json)
    .execute(pool)
    .await?;
    Ok(())
}
