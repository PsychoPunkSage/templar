#![allow(dead_code)]
//! Core types for the Phase 4 render pipeline.
//!
//! RenderStatus — job lifecycle states.
//! RenderJobPayload — Redis queue message.
//! RenderParams — full parameters for LaTeX document generation.
//! TectonicResult — output from a successful Tectonic compilation.
//! RenderError — all failure modes for the pipeline.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::layout::FontFamily;

// ────────────────────────────────────────────────────────────────────────────
// Render job lifecycle
// ────────────────────────────────────────────────────────────────────────────

/// Job lifecycle states stored in `render_jobs.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderStatus {
    Queued,
    Processing,
    Done,
    Failed,
}

impl RenderStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            RenderStatus::Queued => "queued",
            RenderStatus::Processing => "processing",
            RenderStatus::Done => "done",
            RenderStatus::Failed => "failed",
        }
    }
}

impl std::fmt::Display for RenderStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Redis queue payload
// ────────────────────────────────────────────────────────────────────────────

/// Payload stored in the Redis render queue.
/// Worker resolves all other data from the DB using `job_id`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderJobPayload {
    pub job_id: Uuid,
    pub resume_id: Uuid,
}

// ────────────────────────────────────────────────────────────────────────────
// Render parameters
// ────────────────────────────────────────────────────────────────────────────

/// Full set of parameters needed to build a LaTeX document.
/// Derived from the resume row + resume_bullets + page_config defaults.
#[derive(Debug, Clone)]
pub struct RenderParams {
    pub resume_id: Uuid,
    pub font: FontFamily,
    pub font_size_pt: u8,
    pub margin_left_in: f32,
    pub margin_right_in: f32,
    pub sections: Vec<ResumeSection>,
}

/// A named resume section with its ordered bullet texts.
#[derive(Debug, Clone)]
pub struct ResumeSection {
    pub name: String,
    pub bullets: Vec<String>,
}

// ────────────────────────────────────────────────────────────────────────────
// Tectonic output
// ────────────────────────────────────────────────────────────────────────────

/// Output from a successful Tectonic LaTeX → PDF compilation.
#[derive(Debug)]
pub struct TectonicResult {
    /// Raw PDF bytes (stdout from tectonic -).
    pub pdf_bytes: Vec<u8>,
    /// Captured stderr (warnings, font loading messages, etc.).
    pub stderr: String,
    /// Wall-clock time for the compilation.
    pub duration_ms: u64,
}

// ────────────────────────────────────────────────────────────────────────────
// Error type
// ────────────────────────────────────────────────────────────────────────────

/// All failure modes for the render pipeline.
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("Tectonic not found on PATH — install tectonic before starting the server")]
    TectonicNotFound,

    #[error("Tectonic compilation failed (exit={exit_code}): {stderr}")]
    CompilationFailed { exit_code: i32, stderr: String },

    #[error("Tectonic returned empty PDF output")]
    EmptyPdf,

    #[error("S3 upload failed: {0}")]
    S3Upload(String),

    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),

    #[error("Redis error: {0}")]
    Redis(#[from] redis::RedisError),

    #[error("Resume {0} not found")]
    ResumeNotFound(Uuid),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_status_as_str_all_variants() {
        assert_eq!(RenderStatus::Queued.as_str(), "queued");
        assert_eq!(RenderStatus::Processing.as_str(), "processing");
        assert_eq!(RenderStatus::Done.as_str(), "done");
        assert_eq!(RenderStatus::Failed.as_str(), "failed");
    }

    #[test]
    fn test_render_status_display() {
        assert_eq!(RenderStatus::Queued.to_string(), "queued");
        assert_eq!(RenderStatus::Processing.to_string(), "processing");
        assert_eq!(RenderStatus::Done.to_string(), "done");
        assert_eq!(RenderStatus::Failed.to_string(), "failed");
    }

    #[test]
    fn test_render_error_display_non_empty() {
        let errors: Vec<String> = vec![
            RenderError::TectonicNotFound.to_string(),
            RenderError::CompilationFailed {
                exit_code: 1,
                stderr: "bad latex".to_string(),
            }
            .to_string(),
            RenderError::EmptyPdf.to_string(),
            RenderError::S3Upload("bucket unreachable".to_string()).to_string(),
            RenderError::ResumeNotFound(Uuid::new_v4()).to_string(),
        ];
        for msg in &errors {
            assert!(
                !msg.is_empty(),
                "RenderError display must be non-empty: {msg}"
            );
        }
    }
}
