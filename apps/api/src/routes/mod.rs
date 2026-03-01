pub mod health;

use axum::{
    routing::{get, patch, post},
    Router,
};

use crate::context::handlers as ctx;
use crate::generation::handlers as gen;
use crate::state::AppState;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health_handler))
        // ── Context API (Phase 1) ──────────────────────────────────────────
        .route("/api/v1/context", get(ctx::handle_get_context))
        .route("/api/v1/context/health", get(ctx::handle_context_health))
        .route("/api/v1/context/history", get(ctx::handle_context_history))
        .route("/api/v1/context/version/:v", get(ctx::handle_get_version))
        .route("/api/v1/context/ingest", post(ctx::handle_ingest))
        .route(
            "/api/v1/context/ingest/confirm",
            post(ctx::handle_ingest_confirm),
        )
        .route(
            "/api/v1/context/entries/:id/evergreen",
            patch(ctx::handle_toggle_evergreen),
        )
        // ── Resume / Generation API (Phase 2) ─────────────────────────────
        // Note: specific routes before the :id param route (Axum priority)
        .route("/api/v1/resumes/parse-jd", post(gen::handle_parse_jd))
        .route("/api/v1/resumes/fit-score", post(gen::handle_fit_score))
        .route("/api/v1/resumes/generate", post(gen::handle_generate))
        .route("/api/v1/resumes/:id", get(gen::handle_get_resume))
        // ── Render API (Phase 4) ───────────────────────────────────────────
        .route("/api/v1/render/:job_id", get(not_implemented))
        .route("/api/v1/render/:job_id/status", get(not_implemented))
        .with_state(state)
}

async fn not_implemented() -> Result<(), crate::errors::AppError> {
    Err(crate::errors::AppError::NotImplemented)
}
