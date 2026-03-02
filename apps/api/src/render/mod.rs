// Phase 4: Render Service
// Implements: Tectonic LaTeX → PDF pipeline via Redis async job queue.
// NEVER block the request thread on Tectonic compilation.
#![allow(unused_imports)]

pub mod handlers;
pub mod tectonic;
pub mod templates;
pub mod types;
pub mod worker;

// Re-exports for Phase 5 (grounding) and Phase 6 (frontend) consumers.
pub use types::{RenderError, RenderParams, RenderStatus, ResumeSection, TectonicResult};
pub use worker::RENDER_QUEUE_KEY;
