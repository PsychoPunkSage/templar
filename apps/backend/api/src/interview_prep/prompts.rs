//! LLM prompt templates for interview prep generation.
//!
//! Three prompts:
//!   1. STAR_SCAFFOLD  — per-bullet STAR scaffold + behavioral/technical questions
//!   2. GAP_QUESTIONS  — gap-area questions derived from FitReport.gaps (includes company extraction)
//!
//! Rules applied to every prompt:
//!   - JSON-only output (no markdown fences, no prose outside JSON)
//!   - NEVER use --, ---, or em dashes (—)
//!   - Grounding: only use facts present in the supplied context entry
//!   - Banned scope inflation: team_member context must not generate "I architected / I led / I owned"

// ────────────────────────────────────────────────────────────────────────────
// STAR Scaffold + Questions (per-bullet)
// ────────────────────────────────────────────────────────────────────────────

pub const STAR_SCAFFOLD_SYSTEM: &str = "\
You are an expert interview coach helping candidates prepare for technical and behavioral interviews. \
You MUST respond with valid JSON only. No markdown fences, no explanations outside the JSON. \
Ground every claim strictly in the provided context entry. Do NOT invent facts. \
Do NOT use double dashes (--), triple dashes (---), or em dashes (—) anywhere. \
Write natural, confident prose. Complete sentences. No typographic artifacts.";

/// Template for generating a STAR scaffold and per-bullet questions.
///
/// Placeholders:
///   {bullet_text}        — the resume bullet being processed
///   {context_entry}      — the source context entry (JSON or text)
///   {contribution_type}  — one of: sole_author, primary_contributor, team_member, reviewer
///   {jd_keywords}        — top 8 JD keywords for question targeting
pub const STAR_SCAFFOLD_TEMPLATE: &str = r#"Generate an interview preparation package for this resume bullet.

RESUME BULLET:
{bullet_text}

SOURCE CONTEXT ENTRY (verified facts — use ONLY these):
{context_entry}

CANDIDATE CONTRIBUTION TYPE: {contribution_type}
JD KEYWORDS (target questions toward these): {jd_keywords}

GROUNDING RULES:
1. Every STAR field must be supported by the source context entry above.
2. If the context lacks a specific detail, write a concise placeholder like "details to confirm" rather than inventing facts.
3. Numbers and tool names must appear verbatim from the context entry — do not round, abbreviate, or substitute.
4. If contribution_type is "team_member", the action field must reflect collaborative ownership (e.g. "I contributed to", "I implemented the X component of"). NEVER write "I architected", "I designed and owned", "I led" for team_member entries.

OUTPUT RULES:
- talking_points: 2 to 4 items. Each is a short imperative phrase the candidate should memorise before the interview. No dashes.
- questions: 2 to 4 questions per bullet. Mix behavioral and technical types. Vary question stems.
- All text fields: complete sentences. No dashes (-- or --- or em dash).

Return a JSON object with this EXACT schema (no extra fields):
{
  "star_scaffold": {
    "situation": "...",
    "task": "...",
    "action": "...",
    "result": "...",
    "talking_points": ["...", "..."]
  },
  "questions": [
    {"text": "...", "type": "behavioral"},
    {"text": "...", "type": "technical"}
  ]
}"#;

// ────────────────────────────────────────────────────────────────────────────
// Gap Questions + Company Extraction (single batch call)
// ────────────────────────────────────────────────────────────────────────────

pub const GAP_QUESTIONS_SYSTEM: &str = "\
You are an expert interview coach. Your job is to help candidates prepare for tough questions about \
gaps between their background and the job requirements. \
You MUST respond with valid JSON only. No markdown fences, no prose outside the JSON. \
Do NOT use double dashes (--), triple dashes (---), or em dashes (—). \
Write natural, probing question prose. Complete sentences.";

/// Template for generating gap questions from FitReport gaps and extracting company context.
///
/// Placeholders:
///   {gaps_json}    — JSON array of {area, description} objects from FitReport.gaps
///   {jd_text}      — full JD text (truncated to ~2000 chars) for company/role extraction
pub const GAP_QUESTIONS_TEMPLATE: &str = r#"You have two tasks: generate gap questions and extract company context.

TASK 1 — GAP QUESTIONS:
The candidate is applying for this role but has the following gaps versus the job requirements:

{gaps_json}

For each gap area, produce 3 to 5 interview questions that:
- Probe exactly that gap area (do not deflect to unrelated strengths)
- Are phrased as a tough interviewer would ask them
- Give the candidate the opportunity to show growth, context, or mitigation
- Do NOT use double dashes, triple dashes, or em dashes

TASK 2 — COMPANY CONTEXT EXTRACTION:
From the job description text below, extract: company name, role title, and company stage/size (e.g. "Series B startup", "public enterprise", "mid-size SaaS"). If a field is not determinable, return null.

JOB DESCRIPTION (first 2000 characters):
{jd_text}

Return a JSON object with this EXACT schema (no extra fields):
{
  "gap_questions": [
    {"text": "...", "gap_area": "..."},
    {"text": "...", "gap_area": "..."}
  ],
  "company_name": "...",
  "role_title": "...",
  "company_stage": "..."
}"#;
