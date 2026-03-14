use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use aws_sdk_s3::Client as S3Client;
use bytes::Bytes;
use redis::Client as RedisClient;
use sqlx::PgPool;

use crate::config::Config;
use crate::generation::fit_scoring::FitScorer;
use crate::layout::PageConfig;
use crate::llm_client::LlmClient;
use crate::templates::TemplateCache;

/// Shared application state injected into all route handlers via Axum extractors.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// Redis client for the async render job queue (Phase 4).
    pub redis: RedisClient,
    pub s3: S3Client,
    pub llm: LlmClient,
    pub config: Config,
    /// Pluggable fit scorer. Default: KeywordFitScorer. Swap via ENABLE_LLM_FIT_SCORING env.
    pub fit_scorer: Arc<dyn FitScorer>,
    /// Layout page config — font metrics and page dimensions for the simulation loop.
    /// Phase 3: defaults to Inter at 11pt on US letter with 1" margins.
    pub page_config: PageConfig,

    // ── Phase 8: File-based template system ──────────────────────────────────
    /// In-memory registry of all loaded file-based templates.
    ///
    /// Uses `tokio::sync::RwLock` (async-aware) instead of `std::sync::RwLock`.
    /// std's RwLock blocks the OS thread while held — in an async Tokio context
    /// this starves the executor. The cache is read-heavy (GET /templates on every
    /// page load) and written only once at startup, making RwLock the right choice.
    pub template_cache: Arc<TemplateCache>,

    /// In-memory cache of compiled template PDFs, keyed by template id.
    ///
    /// Populated lazily on the first GET /api/v1/templates/:id/render-pdf request.
    /// RwLock chosen over Mutex: many concurrent readers (no eviction path in MVP),
    /// single writer per unique template id (first request wins via write lock).
    pub template_pdf_cache: Arc<tokio::sync::RwLock<HashMap<String, Bytes>>>,

    /// In-memory cache of compiled thumbnail PDFs, keyed by template id.
    ///
    /// Populated lazily on the first GET /api/v1/templates/:id/thumbnail-pdf request.
    /// Mirrors `template_pdf_cache` but targets `thumbnail.pdf` / `thumbnail.tex`.
    pub template_thumbnail_pdf_cache: Arc<tokio::sync::RwLock<HashMap<String, Bytes>>>,

    /// Filesystem path where template directories are loaded from.
    /// Read from `TEMPLATES_DIR` env (default: `./templates`).
    /// Used by handle_template_render_pdf to locate pre-compiled preview.pdf files.
    pub templates_dir: PathBuf,
}
