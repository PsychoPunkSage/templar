// Phase 4: Render Service
// Implements: pdflatex (TeX Live) LaTeX → PDF pipeline via Redis async job queue.
// NEVER block the request thread on pdflatex compilation.
#![allow(unused_imports)]

pub mod escape;
pub mod handlers;
pub mod pdflatex;
pub mod section_order;
pub mod types;
pub mod worker;

// Re-exports for Phase 5 (grounding) and Phase 6 (frontend) consumers.
pub use escape::escape_latex;
pub use section_order::order_sections;
pub use types::{PdflatexResult, RenderError, RenderParams, RenderStatus, ResumeSection};
pub use worker::RENDER_QUEUE_KEY;
