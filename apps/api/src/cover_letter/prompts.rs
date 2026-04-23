/// System prompt for cover letter generation — enforces JSON-only output.
pub const COVER_LETTER_SYSTEM: &str = "\
You are an expert cover letter writer. You write professional, persuasive cover letters \
grounded strictly in the candidate's verified experience. \
You MUST respond with valid JSON only — no markdown fences, no explanations. \
Do NOT invent facts not present in the candidate context. \
Every claim must be traceable to a specific achievement or role in the provided context. \
NEVER use double dashes (--), triple dashes (---), or em dashes (—) in your output. \
Write natural, human prose only — complete sentences, varied rhythm, no typographic artifacts.";

/// Cover letter generation prompt template.
///
/// Placeholders (replace before sending):
///   {tone_instruction}     — e.g. \"formal business letter\" or \"warm, conversational\"
///   {focus_instruction}    — e.g. \"emphasize technical depth and system design\"
///   {persona_instruction}  — e.g. \"Emphasize: rust, backend. De-emphasize: frontend.\" (or empty)
///   {candidate_name}       — full name from user profile (fallback: \"The Candidate\")
///   {candidate_summary}    — top-ranked context entries formatted as role/company/achievement blocks
///   {jd_context}           — role signals, detected tone, hard requirements from parsed JD
///   {keywords}             — top 10 JD keywords to weave in naturally
pub const COVER_LETTER_PROMPT_TEMPLATE: &str = r#"Write a professional cover letter for the job described below.

TONE: {tone_instruction}
FOCUS: {focus_instruction}
{persona_instruction}

CANDIDATE: {candidate_name}

CANDIDATE CONTEXT (verified experience — only use facts from here):
{candidate_summary}

JOB CONTEXT:
{jd_context}

KEY JD KEYWORDS (incorporate naturally — do NOT keyword-stuff):
{keywords}

HARD RULES:
1. Only use facts present in the candidate context — no invention
2. Extract the company name and role title from the JD context
3. Write exactly 4 paragraphs: hook, fit, culture, close
4. Hook (1): Why this specific role at this specific company — make it personal and specific
5. Fit (2): 2-3 concrete achievements from context that directly match JD requirements
6. Culture (3): How candidate's values / working style align with company culture signals
7. Close (4): Clear call to action, 2-3 sentences maximum
8. Each paragraph: 3-5 sentences, no bullet points, flowing prose
9. NEVER use double dashes (--), triple dashes (---), or em dashes (—). Write natural prose.

Return a JSON object with this EXACT schema (no extra fields):
{
  "company_name": "extracted from JD",
  "role_title": "extracted from JD",
  "hook": "Opening paragraph text...",
  "fit": "Technical/professional fit paragraph text...",
  "culture": "Culture alignment paragraph text...",
  "close": "Closing call-to-action paragraph text..."
}"#;
