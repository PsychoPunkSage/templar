use anyhow::{Context, Result};

// ────────────────────────────────────────────────────────────────────────────
// config.toml deserialization structs
// ────────────────────────────────────────────────────────────────────────────

/// Raw deserialized shape of config.toml.
/// All fields are Option so missing sections/keys fall back to defaults.
#[derive(Debug, Default, serde::Deserialize)]
struct TomlConfig {
    #[serde(default)]
    concurrency: TomlConcurrencyConfig,
    #[serde(default)]
    ingestion: TomlIngestionConfig,
}

#[derive(Debug, Default, serde::Deserialize)]
struct TomlConcurrencyConfig {
    ingest_worker_count: Option<usize>,
    ingest_llm_concurrency: Option<usize>,
    generation_llm_concurrency: Option<usize>,
    layout_llm_concurrency: Option<usize>,
    grounding_llm_concurrency: Option<usize>,
    render_worker_count: Option<usize>,
    generation_worker_count: Option<usize>,
}

#[derive(Debug, Default, serde::Deserialize)]
struct TomlIngestionConfig {
    bullet_token_budget: Option<usize>,
}

/// Attempts to load config.toml from several candidate paths.
///
/// Priority order for path lookup:
///   1. `CONFIG_TOML_PATH` env var (explicit override)
///   2. `config.toml` (current working directory — default in Docker)
///   3. `../config.toml` (one level up — useful in `apps/api/` dev runs)
///   4. `../../config.toml` (two levels up — useful in deep cargo targets)
///
/// If no file is found or parsing fails, logs at INFO/WARN and returns defaults.
/// This function NEVER panics — missing config.toml is gracefully handled.
fn read_toml_config() -> TomlConfig {
    let paths = [
        std::env::var("CONFIG_TOML_PATH").unwrap_or_default(),
        "config.toml".to_string(),
        "../config.toml".to_string(),
        "../../config.toml".to_string(),
    ];
    for path in &paths {
        if path.is_empty() {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(path) {
            match toml::from_str::<TomlConfig>(&content) {
                Ok(cfg) => {
                    tracing::info!(path, "Loaded config.toml");
                    return cfg;
                }
                Err(e) => {
                    tracing::warn!(path, error = %e, "Failed to parse config.toml — using defaults");
                }
            }
        }
    }
    tracing::info!("No config.toml found — using env vars and built-in defaults");
    TomlConfig::default()
}

// ────────────────────────────────────────────────────────────────────────────
// Application configuration
// ────────────────────────────────────────────────────────────────────────────

/// Application configuration loaded from environment variables (+ optional config.toml).
///
/// Priority: env var > config.toml > hardcoded default.
/// Panics at startup if *required* env vars (DATABASE_URL, etc.) are missing.
#[derive(Debug, Clone)]
pub struct Config {
    // ── Required env vars ────────────────────────────────────────────────────
    pub database_url: String,
    pub redis_url: String,
    pub s3_bucket: String,
    pub s3_endpoint: String,
    pub aws_access_key_id: String,
    pub aws_secret_access_key: String,
    pub anthropic_api_key: String,
    pub api_port: u16,
    pub rust_log: String,

    // ── Concurrency tunables (env var > toml > default) ──────────────────────
    /// Number of background Redis ingest workers.
    /// Env: INGEST_WORKER_COUNT  |  Default: 2
    pub ingest_worker_count: usize,

    /// Max concurrent LLM calls across all ingest workers (shared semaphore).
    /// Env: INGEST_LLM_CONCURRENCY  |  Default: 2
    pub ingest_llm_concurrency: usize,

    /// Max concurrent per-entry LLM calls during resume generation.
    /// Env: GENERATION_LLM_CONCURRENCY  |  Default: 3
    pub generation_llm_concurrency: usize,

    /// Max concurrent LLM calls during layout simulation (expand/compress per pass).
    /// Env: LAYOUT_LLM_CONCURRENCY  |  Default: 4
    pub layout_llm_concurrency: usize,

    /// Max concurrent LLM calls during grounding scoring loop.
    /// Env: GROUNDING_LLM_CONCURRENCY  |  Default: 4
    pub grounding_llm_concurrency: usize,

    /// Number of parallel pdflatex render workers.
    /// Env: RENDER_WORKER_COUNT  |  Default: 4
    pub render_worker_count: usize,

    /// Number of parallel background generation workers.
    /// Each worker dequeues a job from Redis and runs the full generate_resume() pipeline.
    /// Keep low (2) — each job already saturates LLM concurrency internally.
    /// Env: GENERATION_WORKER_COUNT  |  Default: 2
    pub generation_worker_count: usize,

    // ── Ingestion tunables ───────────────────────────────────────────────────
    /// Estimated token budget per Phase-B bullet-extraction chunk.
    /// Env: BULLET_TOKEN_BUDGET  |  Default: 1200
    pub bullet_token_budget: usize,

    // ── Auth (optional) ──────────────────────────────────────────────────────
    /// Clerk JWKS URL for JWT verification.
    /// Example: https://<instance>.clerk.accounts.dev/.well-known/jwks.json
    /// If not set, auth is disabled — handlers fall back to the seed MVP user.
    /// This allows the existing test suite to run without Clerk credentials.
    /// Env: CLERK_JWKS_URL
    pub clerk_jwks_url: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        dotenvy::dotenv().ok(); // load .env if present; ignore if missing

        let toml = read_toml_config();

        // Helper: env var (parsed as T) → toml Option<T> → hardcoded default.
        fn env_or<T: std::str::FromStr>(var: &str, toml_val: Option<T>, default: T) -> T {
            std::env::var(var)
                .ok()
                .and_then(|s| s.parse().ok())
                .or(toml_val)
                .unwrap_or(default)
        }

        Ok(Config {
            database_url: require_env("DATABASE_URL")?,
            redis_url: require_env("REDIS_URL")?,
            s3_bucket: require_env("S3_BUCKET")?,
            s3_endpoint: require_env("S3_ENDPOINT")?,
            aws_access_key_id: require_env("AWS_ACCESS_KEY_ID")?,
            aws_secret_access_key: require_env("AWS_SECRET_ACCESS_KEY")?,
            anthropic_api_key: require_env("ANTHROPIC_API_KEY")?,
            api_port: std::env::var("API_PORT")
                .unwrap_or_else(|_| "8080".to_string())
                .parse::<u16>()
                .context("API_PORT must be a valid port number")?,
            rust_log: std::env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string()),

            // Concurrency tunables
            ingest_worker_count: env_or(
                "INGEST_WORKER_COUNT",
                toml.concurrency.ingest_worker_count,
                2,
            ),
            ingest_llm_concurrency: env_or(
                "INGEST_LLM_CONCURRENCY",
                toml.concurrency.ingest_llm_concurrency,
                2,
            ),
            generation_llm_concurrency: env_or(
                "GENERATION_LLM_CONCURRENCY",
                toml.concurrency.generation_llm_concurrency,
                3,
            ),
            layout_llm_concurrency: env_or(
                "LAYOUT_LLM_CONCURRENCY",
                toml.concurrency.layout_llm_concurrency,
                4,
            ),
            grounding_llm_concurrency: env_or(
                "GROUNDING_LLM_CONCURRENCY",
                toml.concurrency.grounding_llm_concurrency,
                4,
            ),
            render_worker_count: env_or(
                "RENDER_WORKER_COUNT",
                toml.concurrency.render_worker_count,
                4,
            ),
            generation_worker_count: env_or(
                "GENERATION_WORKER_COUNT",
                toml.concurrency.generation_worker_count,
                2,
            ),

            // Ingestion tunables
            bullet_token_budget: env_or(
                "BULLET_TOKEN_BUDGET",
                toml.ingestion.bullet_token_budget,
                1200,
            ),

            // Auth (optional)
            clerk_jwks_url: std::env::var("CLERK_JWKS_URL").ok().filter(|s| !s.is_empty()),
        })
    }
}

fn require_env(key: &str) -> Result<String> {
    std::env::var(key).with_context(|| format!("Required environment variable '{key}' is not set"))
}
