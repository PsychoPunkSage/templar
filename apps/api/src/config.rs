use anyhow::{Context, Result};

/// Application configuration loaded from environment variables.
/// Panics at startup if required variables are missing.
#[derive(Debug, Clone)]
pub struct Config {
    pub database_url: String,
    pub redis_url: String,
    pub s3_bucket: String,
    pub s3_endpoint: String,
    pub aws_access_key_id: String,
    pub aws_secret_access_key: String,
    pub anthropic_api_key: String,
    pub port: u16,
    pub rust_log: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok(); // load .env if present; ignore if missing

        Ok(Config {
            database_url: require_env("DATABASE_URL")?,
            redis_url: require_env("REDIS_URL")?,
            s3_bucket: require_env("S3_BUCKET")?,
            s3_endpoint: require_env("S3_ENDPOINT")?,
            aws_access_key_id: require_env("AWS_ACCESS_KEY_ID")?,
            aws_secret_access_key: require_env("AWS_SECRET_ACCESS_KEY")?,
            anthropic_api_key: require_env("ANTHROPIC_API_KEY")?,
            port: std::env::var("PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse::<u16>()
                .context("PORT must be a valid port number")?,
            rust_log: std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),
        })
    }
}

fn require_env(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("Required environment variable '{key}' is not set"))
}
