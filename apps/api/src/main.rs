mod config;
mod context;
mod db;
mod errors;
mod generation;
mod grounding;
mod layout;
mod llm_client;
mod models;
mod projects;
mod render;
mod routes;
mod state;
mod templates;

use anyhow::Result;
use aws_config::Region;
use aws_sdk_s3::config::Credentials;
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use std::sync::Arc;

use crate::config::Config;
use crate::context::worker::spawn_context_ingest_worker;
use crate::db::create_pool;
use crate::generation::fit_scoring::{KeywordFitScorer, LlmFitScorer};
use crate::layout::{default_page_config, FontFamily};
use crate::llm_client::LlmClient;
use crate::render::pdflatex::check_pdflatex_available;
use crate::render::worker::spawn_render_worker;
use crate::routes::build_router;
use crate::state::AppState;
use crate::templates::{load_templates_from_dir, precompute_thumbnails, TemplateCache};

#[tokio::main]
async fn main() -> Result<()> {
    // Load configuration first (panics on missing required env vars)
    let config = Config::from_env()?;

    // Initialize structured logging
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new(format!("{}={}", env!("CARGO_PKG_NAME"), &config.rust_log))
        }))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Templar API v{}", env!("CARGO_PKG_VERSION"));

    // Initialize PostgreSQL
    let db = create_pool(&config.database_url).await?;

    // Initialize Redis
    let redis = redis::Client::open(config.redis_url.clone())?;
    info!("Redis client initialized");

    // Initialize S3 / MinIO
    let s3 = build_s3_client(&config).await;
    info!("S3 client initialized");

    // Initialize LLM client
    let llm = LlmClient::new(config.anthropic_api_key.clone());
    info!("LLM client initialized (model: {})", llm_client::MODEL);

    // Initialize fit scorer.
    // Set FIT_SCORER_BACKEND=llm to use semantic Claude-based scoring (Phase 7.0).
    // Default: KeywordFitScorer (fast, deterministic, no LLM call).
    let fit_scorer_backend =
        std::env::var("FIT_SCORER_BACKEND").unwrap_or_else(|_| "keyword".to_string());
    let fit_scorer: Arc<dyn crate::generation::fit_scoring::FitScorer> =
        if fit_scorer_backend == "llm" {
            info!("Fit scorer: LlmFitScorer (semantic, Claude-backed)");
            Arc::new(LlmFitScorer(llm.clone()))
        } else {
            info!("Fit scorer: KeywordFitScorer (default)");
            Arc::new(KeywordFitScorer)
        };

    // Initialize layout page config (Phase 3: Inter 11pt on US letter, 1" margins)
    let page_config = default_page_config(FontFamily::Inter);
    info!(
        "Layout page config: {:?} {}pt",
        page_config.font, page_config.font_size_pt
    );

    // Load file-based templates from TEMPLATES_DIR (default: ./templates).
    // This is synchronous and cheap — just reads a handful of small text files.
    // A missing directory is not fatal (see load_templates_from_dir docstring).
    let templates_dir: PathBuf = std::env::var("TEMPLATES_DIR")
        .unwrap_or_else(|_| "./templates".to_string())
        .into();
    let loaded = load_templates_from_dir(&templates_dir)
        .map_err(|e| anyhow::anyhow!("Failed to load templates: {e}"))?;
    let template_count = loaded.len();
    let template_cache = Arc::new(TemplateCache::new(loaded));
    info!(
        "Templates loaded: {} template(s) from '{}'",
        template_count,
        templates_dir.display()
    );

    // Build app state
    let state = AppState {
        db,
        redis,
        s3,
        llm,
        config: config.clone(),
        fit_scorer,
        page_config,
        template_cache: template_cache.clone(),
        template_pdf_cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
        template_thumbnail_pdf_cache: Arc::new(tokio::sync::RwLock::new(
            std::collections::HashMap::new(),
        )),
        templates_dir: templates_dir.clone(),
    };

    // Check pdflatex binary is available on PATH (fail fast at startup)
    check_pdflatex_available()
        .await
        .map_err(|e| anyhow::anyhow!("pdflatex not available: {e}"))?;
    info!("pdflatex render engine: available");

    // Spawn thumbnail pre-computation as a fire-and-forget background task.
    // Does NOT block server readiness — server answers requests immediately while
    // thumbnails are being generated. If it fails (e.g. pdftoppm not installed
    // in dev), it logs warnings but the server continues normally.
    {
        let cache_clone = template_cache.clone();
        let s3_clone = state.s3.clone();
        let bucket_clone = state.config.s3_bucket.clone();
        tokio::spawn(async move {
            precompute_thumbnails(cache_clone, s3_clone, bucket_clone).await;
        });
    }
    info!("Thumbnail pre-computation: task spawned");

    // Spawn background render worker (clones before state is moved into router)
    spawn_render_worker(
        state.redis.clone(),
        state.db.clone(),
        state.s3.clone(),
        state.config.s3_bucket.clone(),
        // Pass template cache so worker can use file-based templates
        template_cache,
    );
    info!("Render worker: spawned");

    // Spawn N background context ingest workers (configurable via INGEST_WORKER_COUNT)
    let ingest_worker_count: usize = std::env::var("INGEST_WORKER_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);
    for _ in 0..ingest_worker_count {
        spawn_context_ingest_worker(
            state.redis.clone(),
            state.db.clone(),
            state.llm.clone(),
            state.s3.clone(),
            state.config.s3_bucket.clone(),
        );
    }
    info!("Context ingest workers: spawned {ingest_worker_count}");

    // Build router
    let app = build_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive()); // TODO: tighten CORS in production

    let addr: SocketAddr = format!("0.0.0.0:{}", config.api_port).parse()?;
    info!("Listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

/// Constructs an S3 client configured for MinIO (local) or AWS (production).
async fn build_s3_client(config: &Config) -> aws_sdk_s3::Client {
    let credentials = Credentials::new(
        &config.aws_access_key_id,
        &config.aws_secret_access_key,
        None,
        None,
        "templar-static",
    );

    let s3_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
        .region(Region::new("us-east-1"))
        .credentials_provider(credentials)
        .endpoint_url(&config.s3_endpoint)
        .load()
        .await;

    let s3_client_config = aws_sdk_s3::config::Builder::from(&s3_config)
        .force_path_style(true)
        .build();

    aws_sdk_s3::Client::from_conf(s3_client_config)
}
