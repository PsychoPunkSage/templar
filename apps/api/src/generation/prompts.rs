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

/// System prompt for per-entry resume generation — enforces JSON-only output.
pub const PER_ENTRY_GENERATION_SYSTEM: &str =
    "You are an expert resume writer generating factual, \
    grounded resume bullets from a single professional context entry. \
    You MUST respond with valid JSON only — a JSON object with a \"bullets\" array. \
    Do NOT include any text outside the JSON object. \
    Do NOT use markdown code fences. \
    Do NOT invent facts not present in the context entry. \
    Every bullet must be traceable to a specific claim in the raw context text provided.";

/// Per-entry resume generation prompt template.
/// Replace: {grounding_instruction}, {scope_instruction}, {entry_type}, {contribution_type},
///          {allowed_verbs_json}, {entry_data_json}, {raw_text}, {keywords_json},
///          {jd_context}, {entry_fit_context}
pub const PER_ENTRY_GENERATION_PROMPT_TEMPLATE: &str = r#"{grounding_instruction}

{scope_instruction}

ENTRY TYPE: {entry_type}
CONTRIBUTION TYPE: {contribution_type}
ALLOWED VERBS for this contribution level: {allowed_verbs_json}

CONTEXT ENTRY DATA (structured fields):
{entry_data_json}

RAW CONTEXT TEXT (original notes — primary source of truth):
{raw_text}

JD KEYWORDS to incorporate naturally (do NOT keyword-stuff):
{keywords_json}

FULL JD CONTEXT:
{jd_context}

JD FIT FOR THIS ENTRY:
{entry_fit_context}

Generate resume bullets for this single context entry. Return a JSON object:
{{
  "bullets": [
    {{"text": "...", "line_estimate": 1, "jd_keywords_used": ["k8s"]}}
  ]
}}

HARD RULES:
1. Only use facts present in the context entry data or raw text — no invention, no interpolation
2. `line_estimate` must be 1 or 2 — NEVER 3 or more
3. Match `contribution_type` to verb language per the scope instruction above
4. If the JD FIT section above shows no strong or partial matches for this entry, return {{"bullets": []}}
5. Generate 2–4 bullets for experience/project entries; 1–2 for awards/publications
6. Every bullet must begin with a strong action verb from the allowed verbs list"#;

// ────────────────────────────────────────────────────────────────────────────
// Phase 7.0 — LLM-based fit scoring
// ────────────────────────────────────────────────────────────────────────────

/// System prompt for LLM fit scoring.
pub const LLM_FIT_SCORE_SYSTEM: &str =
    "You are scoring a candidate's fit for a job description. \
    Analyze alignment across technical skills, experience level, domain knowledge, and soft skills. \
    You MUST respond with valid JSON only — no markdown fences, no explanations. \
    Be honest: only mark something as a strong match if the evidence is clear and direct.";

/// Fit score prompt template. Placeholder legend:
///   {entries_summary}  — structured per-entry block: metadata + raw_text snippet (≤500 chars/entry)
///   {jd_keywords}      — keyword inventory from ParsedJD (keyword, frequency, weight)
///   {jd_requirements}  — hard requirements list from ParsedJD ([REQUIRED] / [preferred])
///   {jd_text}          — role context from ParsedJD (seniority, culture, nice-to-haves).
///                        NOT raw JD prose. Built by build_jd_role_context() in fit_scoring.rs.
pub const LLM_FIT_SCORE_PROMPT_TEMPLATE: &str = r#"Score this candidate's fit for the job description below.

CANDIDATE CONTEXT SUMMARY:
{entries_summary}

JD KEYWORDS TO CHECK:
{jd_keywords}

JD HARD REQUIREMENTS:
{jd_requirements}

ROLE CONTEXT (seniority, culture signals, nice-to-haves):
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
  "recommendation": "Strong fit for the infrastructure role. Missing GraphQL experience but core Rust/distributed systems background is excellent.",
  "selected_entry_indices": [0, 2]
}

Rules:
- overall_score: integer 0–100 (weighted average of all keyword alignments)
- strong_matches: strength ≥ 0.8 — direct, clear evidence in candidate context
- partial_matches: strength 0.4–0.79 — indirect, partial, or adjacent evidence
- gaps: all JD keywords with strength < 0.4 — nothing relevant in candidate context
- Keep recommendation to 2 sentences maximum
- selected_entry_indices: list of integer indices from the [N] prefix in CANDIDATE CONTEXT SUMMARY — include entries with any match; omit only entries with zero relevance
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
