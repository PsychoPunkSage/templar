#![allow(dead_code)]

// Shared prompt constants and prompt-building utilities.
// Each service that needs LLM calls defines its own prompts.rs alongside it.
// This file contains cross-cutting prompt fragments.

/// System prompt fragment that enforces JSON-only output.
pub const JSON_ONLY_SYSTEM: &str = "You are a precise, structured assistant. \
    You MUST respond with valid JSON only. \
    Do NOT include any text outside the JSON object. \
    Do NOT use markdown code fences. \
    Do NOT include explanations or apologies.";

/// Common instruction appended to all generation prompts.
pub const GROUNDING_INSTRUCTION: &str = "\
    CRITICAL: Every claim you generate must be traceable to a specific source entry ID \
    provided in the context. Do NOT infer, interpolate, or invent details. \
    If the context does not support a claim, omit it entirely. \
    Tag every output bullet with its source `entry_id`.";

/// Instruction to avoid contribution type inflation.
pub const SCOPE_INSTRUCTION: &str = "\
    CRITICAL: Map contribution_type to language precisely: \
    - sole_author / primary_contributor: may use 'Architected', 'Designed', 'Built', 'Led' \
    - team_member: must use 'Contributed to', 'Collaborated on', 'Implemented (as part of team)' \
    - reviewer: must use 'Reviewed', 'Evaluated', 'Assessed' \
    NEVER upgrade a team_member to solo language. This is a hard rule.";
