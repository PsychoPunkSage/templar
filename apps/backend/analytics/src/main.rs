mod config;

use anyhow::Result;
use axum::{http::StatusCode, routing::get, routing::post, Router};
use std::net::SocketAddr;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use crate::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::from_env()?;

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.rust_log)))
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("Starting Templar Analytics Service v{}", env!("CARGO_PKG_VERSION"));
    info!(
        nats_url = %config.nats_url,
        clickhouse_url = %config.clickhouse_url,
        app_env = %config.app_env,
        "Config loaded"
    );

    // Phase 6: NATS consumer will be wired here
    tokio::spawn(async {
        info!("NATS consumer: not yet implemented — Phase 6");
    });

    let app = Router::new()
        .route("/health", get(health))
        .route("/ingest", post(ingest_stub));

    let addr: SocketAddr = format!("0.0.0.0:{}", config.analytics_port).parse()?;
    info!("Analytics service listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health() -> StatusCode {
    StatusCode::OK
}

/// Stub /ingest handler — replaced in Phase 3 (schema layer) + Phase 5 (pipeline).
async fn ingest_stub() -> StatusCode {
    StatusCode::OK
}
