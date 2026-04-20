use anyhow::Result;
use sqlx::PgPool;
use uuid::Uuid;

use crate::personas::PersonaSuggestion;

/// Look up cached persona suggestions for the given (user_id, context_hash, persona_hash) triple.
///
/// Returns `None` on miss. Returns an error only for genuine DB failures —
/// deserialisation failures are treated as a miss so a fresh LLM call runs instead.
///
/// Uses runtime query (not `query!` macro) so the table does not need to be present
/// at compile time — the migration runs at startup.
pub async fn lookup_cache(
    pool: &PgPool,
    user_id: Uuid,
    context_hash: &str,
    persona_hash: &str,
) -> Result<Option<Vec<PersonaSuggestion>>> {
    let row = sqlx::query_as::<_, (serde_json::Value,)>(
        r#"SELECT suggestions FROM persona_suggestions_cache
           WHERE user_id = $1 AND context_hash = $2 AND persona_hash = $3
           LIMIT 1"#,
    )
    .bind(user_id)
    .bind(context_hash)
    .bind(persona_hash)
    .fetch_optional(pool)
    .await?;

    match row {
        Some((json_val,)) => match serde_json::from_value::<Vec<PersonaSuggestion>>(json_val) {
            Ok(suggestions) => Ok(Some(suggestions)),
            Err(_) => Ok(None), // Deserialisation failure → treat as miss
        },
        None => Ok(None),
    }
}

/// Upsert persona suggestions into the cache.
///
/// Uses `ON CONFLICT ... DO UPDATE` so re-generating the same (user, context, persona)
/// triple refreshes the cached value and resets `created_at`.
pub async fn upsert_cache(
    pool: &PgPool,
    user_id: Uuid,
    context_hash: &str,
    persona_hash: &str,
    suggestions: &[PersonaSuggestion],
) -> Result<()> {
    let suggestions_json = serde_json::to_value(suggestions)?;
    sqlx::query(
        r#"INSERT INTO persona_suggestions_cache (user_id, context_hash, persona_hash, suggestions)
           VALUES ($1, $2, $3, $4)
           ON CONFLICT ON CONSTRAINT uq_persona_suggestion_cache
           DO UPDATE SET suggestions = EXCLUDED.suggestions, created_at = NOW()"#,
    )
    .bind(user_id)
    .bind(context_hash)
    .bind(persona_hash)
    .bind(suggestions_json)
    .execute(pool)
    .await?;
    Ok(())
}
