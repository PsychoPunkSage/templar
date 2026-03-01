mod config;
mod context;
mod db;
mod errors;
mod generation;
mod grounding;
mod layout;
mod llm_client;
mod models;
mod render;
mod routes;
mod state;

use anyhow::Result;
use aws_config::Region;
use aws_sdk_s3::config::Credentials;
use std::net::SocketAddr;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use std::sync::Arc;

use crate::config::Config;
use crate::db::create_pool;
use crate::generation::fit_scoring::KeywordFitScorer;
use crate::layout::{default_page_config, FontFamily};
use crate::llm_client::LlmClient;
use crate::routes::build_router;
use crate::state::AppState;

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

    // Initialize fit scorer (KeywordFitScorer by default — swap via ENABLE_LLM_FIT_SCORING)
    let fit_scorer = Arc::new(KeywordFitScorer);

    // Initialize layout page config (Phase 3: Inter 11pt on US letter, 1" margins)
    let page_config = default_page_config(FontFamily::Inter);
    info!(
        "Layout page config: {:?} {}pt",
        page_config.font, page_config.font_size_pt
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
    };

    // Build router
    let app = build_router(state)
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive()); // TODO: tighten CORS in production

    let addr: SocketAddr = format!("0.0.0.0:{}", config.port).parse()?;
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

    aws_sdk_s3::Client::new(&s3_config)
}
