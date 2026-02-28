use anyhow::Result;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use tracing::info;

/// Creates and returns a PostgreSQL connection pool.
pub async fn create_pool(database_url: &str) -> Result<PgPool> {
    info!("Connecting to PostgreSQL...");

    let pool = PgPoolOptions::new()
        .max_connections(10)
        .connect(database_url)
        .await?;

    info!("PostgreSQL connection pool established");
    Ok(pool)
}
