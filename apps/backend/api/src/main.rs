mod auth;
mod config;
mod context;
mod cover_letter;
mod db;
mod errors;
mod generation;
mod grounding;
mod interview_prep;
mod layout;
mod llm_client;
mod metrics;
mod models;
mod personas;
mod profile;
mod projects;
mod render;
mod routes;
mod state;
mod templates;

use anyhow::Result;
use aws_config::Region;
use aws_sdk_s3::config::Credentials;
use axum::routing::get;
use axum_prometheus::PrometheusMetricLayerBuilder;
use metrics_exporter_prometheus::{Matcher, PrometheusBuilder};
use std::net::SocketAddr;
use std::path::PathBuf;
use tower_http::{cors::CorsLayer, trace::TraceLayer};
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use std::sync::Arc;

use crate::auth::{fetch_jwks, JwksCache};
use crate::config::Config;
use crate::context::worker::spawn_context_ingest_worker;
use crate::db::create_pool;
use crate::generation::fit_scoring::LlmFitScorer;
use crate::generation::worker::spawn_generation_worker;
use crate::interview_prep::job::spawn_interview_prep_worker;
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

    // Install Prometheus metrics recorder with custom histogram bucket boundaries.
    // Must happen before any metrics are recorded (before workers are spawned).
    let (prometheus_layer, metric_handle) = PrometheusMetricLayerBuilder::new()
        .with_metrics_from_fn(|| {
            PrometheusBuilder::new()
                .set_buckets_for_metric(
                    Matcher::Full("generation_duration_seconds".to_string()),
                    &[5.0, 10.0, 20.0, 30.0, 60.0, 90.0, 120.0, 180.0, 300.0, 600.0],
                )
                .expect("generation_duration_seconds buckets")
                .set_buckets_for_metric(
                    Matcher::Full("render_duration_seconds".to_string()),
                    &[0.5, 1.0, 2.0, 5.0, 10.0, 30.0, 60.0],
                )
                .expect("render_duration_seconds buckets")
                .set_buckets_for_metric(
                    Matcher::Full("grounding_score".to_string()),
                    &[0.5, 0.65, 0.70, 0.80, 0.85, 0.90, 1.0],
                )
                .expect("grounding_score buckets")
                .set_buckets_for_metric(
                    Matcher::Full("layout_pass_count".to_string()),
                    &[1.0, 2.0, 3.0],
                )
                .expect("layout_pass_count buckets")
                .install_recorder()
                .expect("failed to install Prometheus recorder")
        })
        .build_pair();
    info!("Prometheus metrics recorder installed");

    // Initialize PostgreSQL
    let db = create_pool(&config.database_url).await?;

    // Background task: sample sqlx pool gauges every 10 seconds.
    {
        let pool_clone = db.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(10));
            loop {
                interval.tick().await;
                crate::metrics::set_db_pool_active(pool_clone.size());
                crate::metrics::set_db_pool_idle(pool_clone.num_idle() as u32);
            }
        });
    }

    // Initialize Redis
    let redis = redis::Client::open(config.redis_url.clone())?;
    info!("Redis client initialized");

    // Initialize S3 / MinIO
    let s3 = build_s3_client(&config).await;
    info!("S3 client initialized");

    // Initialize LLM client
    let llm = LlmClient::new(config.anthropic_api_key.clone());
    info!("LLM client initialized (model: {})", llm_client::MODEL);

    // Fit scorer: always LlmFitScorer (semantic, Claude-backed).
    // KeywordFitScorer has been removed — it returned empty selected_entry_ids which
    // disabled the entry filter in call_llm_with_retry, making JD-aware selection a no-op.
    let fit_scorer: Arc<dyn crate::generation::fit_scoring::FitScorer> = {
        info!("Fit scorer: LlmFitScorer (semantic, Claude-backed)");
        Arc::new(LlmFitScorer(llm.clone()))
    };

    // Initialize Clerk JWKS cache (Phase 9: optional auth)
    let jwks_cache = JwksCache::new();
    if let Some(url) = &config.clerk_jwks_url {
        let keys = fetch_jwks(url).await;
        let mut guard = jwks_cache.0.write().await;
        guard.extend(keys);
        info!("Clerk JWKS loaded: {} key(s)", guard.len());
    } else {
        info!("CLERK_JWKS_URL not set — auth disabled (dev/test mode)");
    }

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
        jwks_cache,
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

    // Spawn background generation worker (FIX-08: decouples generation from HTTP request)
    spawn_generation_worker(
        state.redis.clone(),
        state.db.clone(),
        state.llm.clone(),
        state.fit_scorer.clone(),
        state.page_config.clone(),
        state.config.clone(),
    );
    info!(
        generation_workers = state.config.generation_worker_count,
        "Generation worker: spawned"
    );

    // Shared semaphore for all ingest workers — caps total concurrent LLM calls.
    let ingest_sem = Arc::new(tokio::sync::Semaphore::new(config.ingest_llm_concurrency));

    // Spawn N background context ingest workers (from config: ingest_worker_count)
    for _ in 0..config.ingest_worker_count {
        spawn_context_ingest_worker(
            state.redis.clone(),
            state.db.clone(),
            state.llm.clone(),
            state.s3.clone(),
            state.config.s3_bucket.clone(),
            Arc::clone(&ingest_sem),
            config.bullet_token_budget,
        );
    }
    info!(
        ingest_workers = config.ingest_worker_count,
        ingest_llm_concurrency = config.ingest_llm_concurrency,
        generation_llm_concurrency = config.generation_llm_concurrency,
        layout_llm_concurrency = config.layout_llm_concurrency,
        grounding_llm_concurrency = config.grounding_llm_concurrency,
        render_workers = config.render_worker_count,
        generation_workers = config.generation_worker_count,
        bullet_token_budget = config.bullet_token_budget,
        "Concurrency config loaded"
    );
    info!(
        "Context ingest workers: spawned {}",
        config.ingest_worker_count
    );

    // Spawn interview prep workers
    let interview_prep_worker_count: usize = std::env::var("INTERVIEW_PREP_WORKER_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    spawn_interview_prep_worker(
        state.redis.clone(),
        state.db.clone(),
        state.llm.clone(),
        interview_prep_worker_count,
    );
    info!(
        interview_prep_workers = interview_prep_worker_count,
        "Interview prep workers: spawned"
    );

    // Build router — /metrics served on the same port as the API.
    // Prometheus scrapes this endpoint from within the Docker monitoring network.
    let app = build_router(state)
        .route("/metrics", get(move || async move { metric_handle.render() }))
        .layer(prometheus_layer)
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
