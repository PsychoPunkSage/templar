#![allow(dead_code)]

// All LLM prompt constants for the Generation module.
// Reuses cross-cutting fragments from llm_client::prompts.

/// System prompt for JD parsing — enforces JSON-only output.
pub const JD_PARSE_SYSTEM: &str =
    "You are an expert job description analyst and resume strategist. \
    Parse a job description and extract structured information. \
    You MUST respond with valid JSON only. \
    Do NOT include any text outside the JSON object. \
    Do NOT use markdown code fences. \
    Do NOT include explanations or apologies.";

/// JD parsing prompt template. Replace `{jd_text}` before sending.
pub const JD_PARSE_PROMPT_TEMPLATE: &str = r#"Parse the following job description and extract structured information.

Return a JSON object with this EXACT schema (no extra fields):
{
  "hard_requirements": [
    {"text": "5+ years Rust programming", "is_required": true}
  ],
  "soft_signals": [
    "Kubernetes experience a plus"
  ],
  "role_signals": {
    "is_startup": false,
    "is_ic_focused": true,
    "is_research": false,
    "seniority": "senior"
  },
  "keyword_inventory": [
    {
      "keyword": "Rust",
      "frequency": 5,
      "position_weight": 0.8,
      "weighted_score": 4.0
    }
  ],
  "detected_tone": "CollaborativeEnterprise"
}

Rules for parsing:

POSITION WEIGHTS for keyword scoring:
- Title / header: 1.0
- Requirements section ("Required:", "Must have:", "You need:"): 0.8
- Responsibilities section ("You will:", "Responsibilities:"): 0.6
- About Us / company section: 0.3
weighted_score = frequency * position_weight

TONE OPTIONS (pick exactly one):
- "AggressiveStartup": fast-paced language — "own", "drive", "move fast", "spearhead", "disrupt"
- "CollaborativeEnterprise": team-oriented — "partner", "collaborate", "contribute", "support teams"
- "ResearchOriented": academic / investigative — "investigate", "publish", "evaluate", "propose"
- "ProductOriented": shipping focus — "ship", "launch", "deliver", "improve user experience"

HARD REQUIREMENTS: Explicit must-haves — phrases like "required", "must have", "you will need", minimum years.
SOFT SIGNALS: Nice-to-haves — phrases like "preferred", "bonus", "nice to have", "plus".

SENIORITY: "junior", "mid", "senior", "staff", "principal", "director", or "unknown".

Extract ALL meaningful technical keywords (languages, frameworks, tools, concepts) and score them.

JOB DESCRIPTION:
{jd_text}"#;

/// System prompt for resume generation — enforces JSON-only output.
pub const GENERATION_SYSTEM: &str = "You are an expert resume writer generating factual, \
    grounded resume bullets from verified professional context. \
    You MUST respond with valid JSON only — a JSON array of bullet objects. \
    Do NOT include any text outside the JSON array. \
    Do NOT use markdown code fences. \
    Do NOT invent facts not present in the context entries.";

/// Resume generation prompt template.
/// Replace: {grounding_instruction}, {scope_instruction}, {tone_json},
///          {entries_json}, {keywords_json}, {jd_summary}
pub const GENERATION_PROMPT_TEMPLATE: &str = r#"{grounding_instruction}

{scope_instruction}

TONE CALIBRATION for this role:
{tone_json}

SELECTED CONTEXT ENTRIES (source of truth — ONLY use facts from these):
{entries_json}

JD KEYWORDS to incorporate naturally (do NOT keyword-stuff):
{keywords_json}

JOB DESCRIPTION SUMMARY:
{jd_summary}

Generate resume bullets for each relevant context entry. Return a JSON ARRAY:
[
  {
    "text": "Architected distributed caching layer reducing p99 latency by 40% across 3 services",
    "source_entry_id": "the-exact-entry_id-uuid-from-context",
    "section": "experience",
    "line_estimate": 1,
    "jd_keywords_used": ["distributed", "latency", "caching"]
  }
]

HARD RULES:
1. EVERY bullet MUST have `source_entry_id` matching one of the entry_id values above — no exceptions
2. `line_estimate` must be 1 or 2 — NEVER 3 or more
3. Use ONLY facts from the context entries — no interpolation, no invention
4. Match `contribution_type` to language exactly per the scope instruction above
5. Pack information densely — one strong bullet per entry, two if the entry is rich enough
6. Incorporate JD keywords naturally where they appear in the context — never force-fit
7. Do NOT include bullets for entries with no relevant content for this role"#;

// ────────────────────────────────────────────────────────────────────────────
// Phase 7.0 — LLM-based fit scoring
// ────────────────────────────────────────────────────────────────────────────

/// System prompt for LLM fit scoring.
pub const LLM_FIT_SCORE_SYSTEM: &str =
    "You are scoring a candidate's fit for a job description. \
    Analyze alignment across technical skills, experience level, domain knowledge, and soft skills. \
    You MUST respond with valid JSON only — no markdown fences, no explanations. \
    Be honest: only mark something as a strong match if the evidence is clear and direct.";

/// LLM fit score prompt template.
/// Replace: {entries_summary}, {jd_keywords}, {jd_requirements}, {jd_text}
pub const LLM_FIT_SCORE_PROMPT_TEMPLATE: &str = r#"Score this candidate's fit for the job description below.

CANDIDATE CONTEXT SUMMARY:
{entries_summary}

JD KEYWORDS TO CHECK:
{jd_keywords}

JD HARD REQUIREMENTS:
{jd_requirements}

JOB DESCRIPTION:
{jd_text}

Return a JSON object with this EXACT schema:
{
  "overall_score": 72,
  "strong_matches": [
    {"dimension": "Rust", "context_evidence": "5 years Rust at Acme", "jd_requirement": "5+ years Rust", "strength": 0.95}
  ],
  "partial_matches": [
    {"dimension": "Kubernetes", "context_evidence": "basic k8s usage mentioned", "jd_requirement": "Production Kubernetes experience", "strength": 0.55}
  ],
  "gaps": [
    {"keyword": "GraphQL", "jd_frequency": 3, "suggestion": null}
  ],
  "recommendation": "Strong fit for the infrastructure role. Missing GraphQL experience but core Rust/distributed systems background is excellent."
}

Rules:
- overall_score: integer 0–100 (weighted average of all keyword alignments)
- strong_matches: strength ≥ 0.8 — direct, clear evidence in candidate context
- partial_matches: strength 0.4–0.79 — indirect, partial, or adjacent evidence
- gaps: all JD keywords with strength < 0.4 — nothing relevant in candidate context
- Keep recommendation to 2 sentences maximum
- Do NOT include any text outside the JSON object"#;

/// Reframe hint prompt template.
/// Replace: {entry_json}, {tone}, {jd_summary}
pub const REFRAME_PROMPT_TEMPLATE: &str = r#"Given this context entry and the detected JD tone, suggest a concise alternative framing that better highlights the most relevant aspect of this entry for the target role.

Context entry:
{entry_json}

Detected JD tone: {tone}
Job focus: {jd_summary}

Return a JSON object:
{
  "suggested_framing": "Brief framing hint — e.g. 'position as infrastructure scale story emphasizing reliability'"
}"#;
