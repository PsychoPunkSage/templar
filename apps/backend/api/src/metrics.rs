//! Observability helpers — thin wrappers around the `metrics` crate macros.
//!
//! All metrics land in the same registry that `PrometheusMetricLayer` installs,
//! so they appear alongside the HTTP request metrics at GET /metrics.
//!
//! Histogram bucket boundaries are configured at recorder-installation time in
//! main.rs via `metrics_exporter_prometheus::PrometheusBuilder`.

// ── Gauge recording ───────────────────────────────────────────────────────────

pub fn set_generation_queue_depth(depth: i64) {
    metrics::gauge!("generation_queue_depth").set(depth as f64);
}

pub fn set_render_queue_depth(depth: i64) {
    metrics::gauge!("render_queue_depth").set(depth as f64);
}

pub fn set_db_pool_active(count: u32) {
    metrics::gauge!("db_pool_connections_active").set(count as f64);
}

pub fn set_db_pool_idle(count: u32) {
    metrics::gauge!("db_pool_connections_idle").set(count as f64);
}

// ── Histogram recording ───────────────────────────────────────────────────────

pub fn observe_generation_duration(elapsed_secs: f64, status: &'static str) {
    metrics::histogram!("generation_duration_seconds", "status" => status).record(elapsed_secs);
}

pub fn observe_render_duration(elapsed_secs: f64, status: &'static str) {
    metrics::histogram!("render_duration_seconds", "status" => status).record(elapsed_secs);
}

pub fn observe_layout_pass_count(passes: u8) {
    metrics::histogram!("layout_pass_count").record(passes as f64);
}

pub fn observe_grounding_score(composite: f32, verdict: &'static str) {
    metrics::histogram!("grounding_score", "verdict" => verdict).record(composite as f64);
}
