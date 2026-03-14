//! Axum handlers for the template API.
//!
//! GET /api/v1/templates              — list all available templates (no DB, no alloc in hot path)
//! GET /api/v1/templates/:id/preview  — proxy thumbnail PNG from S3 (404 if unknown template)
//! GET /api/v1/templates/:id/render-pdf — compile template LaTeX → PDF; response cached in-memory

use aws_sdk_s3::Client as S3Client;
use axum::{
    body::{Body, Bytes},
    extract::{Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::state::AppState;
use crate::templates::TemplateSummary;

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/templates
// ────────────────────────────────────────────────────────────────────────────

/// Lists all available resume templates.
///
/// Performance: reads from the in-memory TemplateCache (RwLock read lock).
/// No DB query, no network call — expected < 1ms even with many templates.
///
/// Returns a `Cache-Control: public, max-age=300` header so the CDN or browser
/// can cache the list for 5 minutes. Adding a new template still requires an
/// API restart, so cache staleness is bounded by the deploy window.
pub async fn handle_list_templates(State(state): State<AppState>) -> impl IntoResponse {
    // Acquire a shared (read) lock — allows concurrent readers with no contention
    // as long as no writer is active. The template cache is written only once at
    // startup, so in practice this lock is always immediately granted.
    let cache = state.template_cache.read().await;

    let mut summaries: Vec<TemplateSummary> = cache
        .values()
        .map(|t| TemplateSummary {
            id: t.metadata.id.clone(),
            name: t.metadata.name.clone(),
            description: t.metadata.description.clone(),
            tags: t.metadata.tags.clone(),
            // Preview URL is relative to the API — frontend prepends API_BASE.
            // This avoids leaking S3 endpoint URLs to the browser, which would
            // only work in dev (minio hostname unreachable from host browser).
            thumbnail_url: format!("/api/v1/templates/{}/preview", t.metadata.id),
        })
        .collect();

    // Sort alphabetically by id for stable ordering across restarts
    summaries.sort_by(|a, b| a.id.cmp(&b.id));

    debug!(count = summaries.len(), "Returning template list");

    (
        [
            (header::CACHE_CONTROL, "public, max-age=300"),
            (header::CONTENT_TYPE, "application/json"),
        ],
        Json(serde_json::json!({ "templates": summaries })),
    )
}

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/templates/:id/preview
// ────────────────────────────────────────────────────────────────────────────

/// Proxies the thumbnail PNG for a template from S3 to the browser.
///
/// Why proxy instead of returning a direct S3 URL?
///   - In dev, MinIO runs on `minio:9000` (Docker internal hostname). The browser
///     cannot reach that hostname — it would need `localhost:9000`. Proxying through
///     the API works in both dev and production without environment-specific URLs.
///   - The S3 presigned URL approach would add complexity for a PNG that is effectively
///     a static asset. Proxying is simpler for the MVP.
///
/// Returns 404 if the template_id is unknown, 503 if S3 fetch fails.
pub async fn handle_template_preview(
    Path(template_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    debug!(template_id = %template_id, "Template preview (thumbnail) requested");

    // Validate template exists in cache before hitting S3
    let thumbnail_key = {
        let cache = state.template_cache.read().await;
        match cache.get(&template_id) {
            Some(t) => t.metadata.thumbnail_s3_key.clone(),
            None => {
                warn!(template_id = %template_id, "Template not found in cache for thumbnail request");
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "message": format!("Template '{}' not found", template_id) }
                    })),
                )
                    .into_response();
            }
        }
    };

    // Fetch thumbnail from S3
    match fetch_s3_png(&state.s3, &state.config.s3_bucket, &thumbnail_key).await {
        Ok(bytes) => {
            // Return PNG bytes with appropriate headers
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "image/png")
                // Cache 1 day — thumbnail is regenerated only when template changes (on restart)
                .header(header::CACHE_CONTROL, "public, max-age=86400")
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
            warn!(template_id = %template_id, key = %thumbnail_key, error = %e, "S3 thumbnail fetch failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(serde_json::json!({
                    "error": { "message": "Thumbnail not available yet — generation may be in progress" }
                })),
            )
                .into_response()
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/templates/:id/render-pdf
// ────────────────────────────────────────────────────────────────────────────

/// Compiles a template's LaTeX source to PDF and returns the raw bytes.
///
/// Results are cached in `state.template_pdf_cache` after the first compile.
/// Cache is in-process memory — it is invalidated only on restart. This is
/// acceptable for the MVP because template `.tex` files change only on deploy.
///
/// Error conditions:
/// - 404: template id not found in the template cache
/// - 500: pdflatex compilation failed (stderr included in the error body)
pub async fn handle_template_render_pdf(
    Path(template_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    info!(template_id = %template_id, "Template render-pdf request received");

    // Fast path: already compiled — return from cache
    {
        let cache = state.template_pdf_cache.read().await;
        if let Some(pdf_bytes) = cache.get(&template_id) {
            debug!(template_id = %template_id, pdf_bytes = pdf_bytes.len(), "Template PDF cache HIT — serving from memory");
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/pdf")
                .header(header::CACHE_CONTROL, "public, max-age=3600")
                .body(Body::from(pdf_bytes.clone()))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }
    debug!(template_id = %template_id, "Template PDF cache MISS — proceeding to load source");

    // Read the LaTeX source from the template cache (also validates template exists)
    let latex_source = {
        let cache = state.template_cache.read().await;
        match cache.get(&template_id) {
            Some(t) => t.latex_source.clone(),
            None => {
                warn!(template_id = %template_id, "Template not found in template cache");
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "message": format!("Template '{}' not found", template_id) }
                    })),
                )
                    .into_response();
            }
        }
    };

    // Fast path: serve pre-compiled preview.pdf from disk (built at Docker image build time).
    // This avoids a 5-10s pdflatex compile on every cold request — the preview PDF is
    // compiled once during `docker build` and baked into the image.
    let preview_path = state.templates_dir.join(&template_id).join("preview.pdf");
    debug!(template_id = %template_id, path = %preview_path.display(), "Checking for pre-compiled preview.pdf on disk");
    if preview_path.exists() {
        info!(template_id = %template_id, path = %preview_path.display(), "Serving pre-compiled preview.pdf from disk");
        if let Ok(pdf_bytes) = tokio::fs::read(&preview_path).await {
            let bytes = Bytes::from(pdf_bytes);
            info!(template_id = %template_id, pdf_bytes = bytes.len(), "preview.pdf read from disk successfully — caching in memory");
            {
                let mut cache = state.template_pdf_cache.write().await;
                cache.insert(template_id, bytes.clone());
            }
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/pdf")
                .header(header::CACHE_CONTROL, "public, max-age=3600")
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        } else {
            warn!(template_id = %template_id, path = %preview_path.display(), "preview.pdf exists on disk but could not be read — falling through to pdflatex slow path");
        }
    } else {
        warn!(template_id = %template_id, path = %preview_path.display(), "preview.pdf NOT found on disk — falling through to pdflatex slow path (may take 30-120s)");
    }

    // Slow path: compile via pdflatex — prefer preview-source.tex (contains sample data)
    // over template.tex (which has {{PLACEHOLDER}} tokens that show as literal text in the preview).
    let preview_source_path = state.templates_dir.join(&template_id).join("preview-source.tex");
    let source_to_compile = if preview_source_path.exists() {
        match tokio::fs::read_to_string(&preview_source_path).await {
            Ok(src) => {
                info!(template_id = %template_id, "Using preview-source.tex for pdflatex slow-path compile");
                src
            }
            Err(e) => {
                warn!(template_id = %template_id, error = %e, "Failed to read preview-source.tex — falling back to template.tex");
                latex_source
            }
        }
    } else {
        warn!(template_id = %template_id, "preview-source.tex not found — using template.tex (placeholders will be visible)");
        latex_source
    };

    let job_id = Uuid::new_v4();
    let start = std::time::Instant::now();
    info!(template_id = %template_id, job_id = %job_id, "Starting pdflatex slow-path compile for template preview");
    match crate::render::pdflatex::compile_latex(&source_to_compile, job_id).await {
        Ok(result) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let pdf_bytes = bytes::Bytes::from(result.pdf_bytes);
            info!(
                template_id = %template_id,
                duration_ms = duration_ms,
                pdf_bytes = pdf_bytes.len(),
                "pdflatex template compile succeeded"
            );

            // Store in cache for subsequent requests
            {
                let mut cache = state.template_pdf_cache.write().await;
                cache.insert(template_id, pdf_bytes.clone());
            }

            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/pdf")
                .header(header::CACHE_CONTROL, "public, max-age=3600")
                .body(Body::from(pdf_bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            error!(
                template_id = %template_id,
                duration_ms = duration_ms,
                error = %e,
                "pdflatex template compile FAILED"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "message": format!("Template compilation failed: {e}") }
                })),
            )
                .into_response()
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/templates/:id/thumbnail-pdf
// ────────────────────────────────────────────────────────────────────────────

/// Compiles a template's `thumbnail.tex` to PDF and returns the raw bytes.
///
/// The thumbnail PDF is a compact, card-sized render used in the template picker
/// grid. It differs from `render-pdf` which compiles the full-page `preview-source.tex`.
///
/// Lookup priority (fast → slow):
/// 1. In-memory `template_thumbnail_pdf_cache` (RwLock read) — cache HIT
/// 2. Pre-compiled `thumbnail.pdf` on disk (baked into Docker image at build time)
/// 3. Slow path: compile `thumbnail.tex` via pdflatex
/// 4. Graceful degradation: if `thumbnail.tex` is missing, fall back to `template.tex`
///
/// Error conditions:
/// - 404: template id not found
/// - 500: pdflatex compilation failed
pub async fn handle_template_thumbnail_pdf(
    Path(template_id): Path<String>,
    State(state): State<AppState>,
) -> Response {
    info!(template_id = %template_id, "Template thumbnail-pdf request received");

    // Fast path 1: in-memory cache
    {
        let cache = state.template_thumbnail_pdf_cache.read().await;
        if let Some(pdf_bytes) = cache.get(&template_id) {
            debug!(template_id = %template_id, pdf_bytes = pdf_bytes.len(), "Thumbnail PDF cache HIT — serving from memory");
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/pdf")
                .header(header::CACHE_CONTROL, "public, max-age=3600")
                .body(Body::from(pdf_bytes.clone()))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        }
    }
    debug!(template_id = %template_id, "Thumbnail PDF cache MISS — proceeding to load source");

    // Validate template exists (also loads latex_source for fallback path below)
    let latex_source = {
        let cache = state.template_cache.read().await;
        match cache.get(&template_id) {
            Some(t) => t.latex_source.clone(),
            None => {
                warn!(template_id = %template_id, "Template not found in template cache");
                return (
                    StatusCode::NOT_FOUND,
                    Json(serde_json::json!({
                        "error": { "message": format!("Template '{}' not found", template_id) }
                    })),
                )
                    .into_response();
            }
        }
    };

    // Fast path 2: serve pre-compiled thumbnail.pdf from disk (baked in at Docker build time)
    let thumbnail_disk_path = state.templates_dir.join(&template_id).join("thumbnail.pdf");
    debug!(template_id = %template_id, path = %thumbnail_disk_path.display(), "Checking for pre-compiled thumbnail.pdf on disk");
    if thumbnail_disk_path.exists() {
        info!(template_id = %template_id, path = %thumbnail_disk_path.display(), "Serving pre-compiled thumbnail.pdf from disk");
        if let Ok(pdf_bytes) = tokio::fs::read(&thumbnail_disk_path).await {
            let bytes = Bytes::from(pdf_bytes);
            info!(template_id = %template_id, pdf_bytes = bytes.len(), "thumbnail.pdf read from disk — caching in memory");
            {
                let mut cache = state.template_thumbnail_pdf_cache.write().await;
                cache.insert(template_id, bytes.clone());
            }
            return Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/pdf")
                .header(header::CACHE_CONTROL, "public, max-age=3600")
                .body(Body::from(bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response());
        } else {
            warn!(template_id = %template_id, path = %thumbnail_disk_path.display(), "thumbnail.pdf exists on disk but could not be read — falling through to pdflatex slow path");
        }
    } else {
        warn!(template_id = %template_id, path = %thumbnail_disk_path.display(), "thumbnail.pdf NOT found on disk — falling through to pdflatex slow path");
    }

    // Slow path: compile via pdflatex — prefer thumbnail.tex, fall back to template.tex
    let thumbnail_tex_path = state.templates_dir.join(&template_id).join("thumbnail.tex");
    let source_to_compile = if thumbnail_tex_path.exists() {
        match tokio::fs::read_to_string(&thumbnail_tex_path).await {
            Ok(src) => {
                info!(template_id = %template_id, "Using thumbnail.tex for pdflatex slow-path compile");
                src
            }
            Err(e) => {
                warn!(template_id = %template_id, error = %e, "Failed to read thumbnail.tex — falling back to template.tex");
                latex_source
            }
        }
    } else {
        warn!(template_id = %template_id, "thumbnail.tex not found — falling back to template.tex (placeholders will be visible)");
        latex_source
    };

    let job_id = Uuid::new_v4();
    let start = std::time::Instant::now();
    info!(template_id = %template_id, job_id = %job_id, "Starting pdflatex slow-path compile for template thumbnail");
    match crate::render::pdflatex::compile_latex(&source_to_compile, job_id).await {
        Ok(result) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            let pdf_bytes = bytes::Bytes::from(result.pdf_bytes);
            info!(
                template_id = %template_id,
                duration_ms = duration_ms,
                pdf_bytes = pdf_bytes.len(),
                "pdflatex thumbnail compile succeeded"
            );
            {
                let mut cache = state.template_thumbnail_pdf_cache.write().await;
                cache.insert(template_id, pdf_bytes.clone());
            }
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "application/pdf")
                .header(header::CACHE_CONTROL, "public, max-age=3600")
                .body(Body::from(pdf_bytes))
                .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
        }
        Err(e) => {
            let duration_ms = start.elapsed().as_millis() as u64;
            error!(
                template_id = %template_id,
                duration_ms = duration_ms,
                error = %e,
                "pdflatex thumbnail compile FAILED"
            );
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": { "message": format!("Thumbnail compilation failed: {e}") }
                })),
            )
                .into_response()
        }
    }
}

/// Fetches an S3 object and returns its bytes.
async fn fetch_s3_png(s3: &S3Client, bucket: &str, key: &str) -> anyhow::Result<Vec<u8>> {
    let output = s3
        .get_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("S3 get_object failed: {e}"))?;

    let bytes = output
        .body
        .collect()
        .await
        .map_err(|e| anyhow::anyhow!("S3 body read failed: {e}"))?
        .into_bytes();

    Ok(bytes.to_vec())
}
