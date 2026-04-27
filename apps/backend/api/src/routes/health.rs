use axum::Json;
use serde_json::{json, Value};

/// GET /health
/// Returns a simple status object with service version.
pub async fn health_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "version": "0.1.0",
        "service": "templar-api"
    }))
}
