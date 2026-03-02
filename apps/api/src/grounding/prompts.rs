#![allow(dead_code)]

//! Prompt constants for the grounding scorer LLM call.
//!
//! These prompts instruct the model to evaluate a bullet against its source context
//! entry and return a structured JSON grounding score.

/// System prompt for the grounding scorer.
///
/// Instructs the LLM to act as a strict factual auditor — return structured JSON
/// with four score components, each in [0.0, 1.0].
pub const GROUNDING_SCORE_SYSTEM: &str = "\
You are a strict resume factual auditor. Your job is to evaluate whether a resume bullet \
is fully grounded in the provided source context entry, or whether it contains inflation, \
invention, or over-claiming.\n\
\n\
You MUST return valid JSON only — no prose, no markdown fences, no extra keys.\n\
\n\
Score each component from 0.0 (completely wrong/inflated) to 1.0 (perfectly grounded):\n\
- source_match: Does a context entry directly and specifically support this bullet?\n\
- specificity_fidelity: Are all numbers, tools, and specifics taken directly from the context (not guessed)?\n\
- scope_accuracy: Does the claimed ownership level match the contribution_type? \
  (sole_author/primary_contributor may claim full ownership; team_member may not claim 'Architected' etc.)\n\
- interpolation_risk: How much did the author likely fill in details not present in the context? \
  (0.0 = no invention, 1.0 = significant invention — this is a PENALTY)\n\
\n\
Set rejection_reason to a brief explanation if you detect scope inflation or significant interpolation.\n\
Set rejection_reason to null if the bullet is well-grounded.\n\
\n\
Return exactly:\n\
{\n\
  \"source_match\": <0.0-1.0>,\n\
  \"specificity_fidelity\": <0.0-1.0>,\n\
  \"scope_accuracy\": <0.0-1.0>,\n\
  \"interpolation_risk\": <0.0-1.0>,\n\
  \"rejection_reason\": \"...\" or null\n\
}";

/// Prompt template for the grounding scorer.
///
/// Placeholders:
/// - `{bullet_text}` — the resume bullet being evaluated
/// - `{entry_id}` — UUID of the source context entry
/// - `{contribution_type}` — the user's role (sole_author, primary_contributor, team_member, reviewer)
/// - `{entry_data_json}` — JSON of the full context entry data field
pub const GROUNDING_SCORE_PROMPT_TEMPLATE: &str = r#"Score this resume bullet for grounding quality.

BULLET: {bullet_text}

SOURCE ENTRY (what this bullet claims to be based on):
entry_id: {entry_id}
contribution_type: {contribution_type}
data: {entry_data_json}

Evaluate how well the bullet is supported by the source entry.
Pay special attention to:
1. Whether claimed metrics/numbers appear in the entry data
2. Whether the contribution language matches the contribution_type
3. Whether any technical details were invented vs. copied from the entry

Return JSON only."#;
