// Phase 4: Render Service
// Implements: pdflatex (TeX Live) LaTeX → PDF pipeline via Redis async job queue.
// NEVER block the request thread on pdflatex compilation.
#![allow(unused_imports)]

pub mod handlers;
pub mod pdflatex;
pub mod templates;
pub mod types;
pub mod worker;

// Re-exports for Phase 5 (grounding) and Phase 6 (frontend) consumers.
pub use types::{PdflatexResult, RenderError, RenderParams, RenderStatus, ResumeSection};
pub use worker::RENDER_QUEUE_KEY;
