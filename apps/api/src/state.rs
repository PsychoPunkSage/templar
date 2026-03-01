use std::sync::Arc;

use aws_sdk_s3::Client as S3Client;
use redis::Client as RedisClient;
use sqlx::PgPool;

use crate::config::Config;
use crate::generation::fit_scoring::FitScorer;
use crate::layout::PageConfig;
use crate::llm_client::LlmClient;

/// Shared application state injected into all route handlers via Axum extractors.
#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// Redis client reserved for Phase 4 async render job queue.
    #[allow(dead_code)]
    pub redis: RedisClient,
    pub s3: S3Client,
    pub llm: LlmClient,
    pub config: Config,
    /// Pluggable fit scorer. Default: KeywordFitScorer. Swap via ENABLE_LLM_FIT_SCORING env.
    pub fit_scorer: Arc<dyn FitScorer>,
    /// Layout page config — font metrics and page dimensions for the simulation loop.
    /// Phase 3: defaults to Inter at 11pt on US letter with 1" margins.
    pub page_config: PageConfig,
}
