//! Interview Prep module — generates STAR scaffolds, behavioral questions, and gap questions
//! from resume bullets and a job description.
//!
//! Architecture follows the cover_letter module: generator.rs does the heavy lifting,
//! job.rs handles async Redis worker lifecycle, routes.rs wires HTTP handlers.
//!
//! Queue key: `interview_prep:jobs`
//! Env: INTERVIEW_PREP_WORKER_COUNT (default 2)

pub mod generator;
pub mod job;
pub mod models;
pub mod prompts;
pub mod routes;
