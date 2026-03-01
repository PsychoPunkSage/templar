// Phase 2: Resume Generation Engine
// Implements: JD parsing, fit scoring, content selection, tone calibration, generation.
// All LLM calls go through llm_client — no direct Anthropic SDK calls here.

pub mod content_selector;
pub mod fit_scoring;
pub mod generator;
pub mod handlers;
pub mod jd_parser;
pub mod prompts;
pub mod tone;
