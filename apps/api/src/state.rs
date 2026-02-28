use aws_sdk_s3::Client as S3Client;
use redis::Client as RedisClient;
use sqlx::PgPool;

use crate::config::Config;
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
}
