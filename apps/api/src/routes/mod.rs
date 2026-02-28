pub mod health;

use axum::{
    routing::{get, patch, post},
    Router,
};

use crate::context::handlers;
use crate::errors::AppError;
use crate::state::AppState;

async fn not_implemented() -> Result<(), AppError> {
    Err(AppError::NotImplemented)
}

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health_handler))
        // Context API (Phase 1)
        .route("/api/v1/context", get(handlers::handle_get_context))
        .route(
            "/api/v1/context/health",
            get(handlers::handle_context_health),
        )
        .route(
            "/api/v1/context/history",
            get(handlers::handle_context_history),
        )
        .route(
            "/api/v1/context/version/:v",
            get(handlers::handle_get_version),
        )
        .route("/api/v1/context/ingest", post(handlers::handle_ingest))
        .route(
            "/api/v1/context/ingest/confirm",
            post(handlers::handle_ingest_confirm),
        )
        .route(
            "/api/v1/context/entries/:id/evergreen",
            patch(handlers::handle_toggle_evergreen),
        )
        // Resume API (Phase 2)
        .route("/api/v1/resumes", post(not_implemented))
        .route("/api/v1/resumes/:id", get(not_implemented))
        // Render API (Phase 4)
        .route("/api/v1/render/:job_id", get(not_implemented))
        .route("/api/v1/render/:job_id/status", get(not_implemented))
        .with_state(state)
}
