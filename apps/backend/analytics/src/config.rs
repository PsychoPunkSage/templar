use anyhow::{Context, Result};

/// Analytics service configuration loaded from environment variables.
///
/// Required vars will cause startup to fail if missing.
/// Optional vars fall back to the listed defaults.
#[derive(Debug, Clone)]
pub struct Config {
    // ── Required ─────────────────────────────────────────────────────────────
    pub nats_url: String,
    pub clickhouse_url: String,
    pub clickhouse_db: String,
    pub clickhouse_user: String,
    pub clickhouse_password: String,
    pub database_url: String,
    pub redis_url: String,

    // ── Optional with defaults ────────────────────────────────────────────────
    /// HTTP port for the analytics service. Env: ANALYTICS_PORT | Default: 8090
    pub analytics_port: u16,

    /// JetStream stream name. Env: ANALYTICS_NATS_STREAM | Default: "templar_events"
    pub nats_stream: String,

    /// Subject wildcard the stream filters on. Env: ANALYTICS_NATS_SUBJECT | Default: "templar.events.*"
    pub nats_subject: String,

    /// Deployment environment. Env: APP_ENV | Default: "prod"
    pub app_env: String,

    pub rust_log: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok();

        Ok(Config {
            nats_url: require_env("NATS_URL")?,
            clickhouse_url: require_env("CLICKHOUSE_URL")?,
            clickhouse_db: require_env("CLICKHOUSE_DB")?,
            clickhouse_user: require_env("CLICKHOUSE_USER")?,
            clickhouse_password: require_env("CLICKHOUSE_PASSWORD")?,
            database_url: require_env("DATABASE_URL")?,
            redis_url: require_env("REDIS_URL")?,

            analytics_port: std::env::var("ANALYTICS_PORT")
                .unwrap_or_else(|_| "8090".to_string())
                .parse::<u16>()
                .context("ANALYTICS_PORT must be a valid port number")?,

            nats_stream: std::env::var("ANALYTICS_NATS_STREAM")
                .unwrap_or_else(|_| "templar_events".to_string()),

            nats_subject: std::env::var("ANALYTICS_NATS_SUBJECT")
                .unwrap_or_else(|_| "templar.events.*".to_string()),

            app_env: std::env::var("APP_ENV").unwrap_or_else(|_| "prod".to_string()),

            rust_log: std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        })
    }
}

fn require_env(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("Required environment variable '{key}' is not set"))
}
