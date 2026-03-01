//! LLM prompt constants for layout bullet adjustment.
//!
//! Both prompts instruct the model to return `{"text": "..."}` JSON only.
//! Callers deserialize via `llm.call_json::<AdjustedBullet>()`.

// ────────────────────────────────────────────────────────────────────────────
// Expand prompt (bullet too short — 1-line fill < 80%)
// ────────────────────────────────────────────────────────────────────────────

pub const EXPAND_SYSTEM: &str = "\
You are a resume bullet editor. Your task is to expand a resume bullet that does not \
fill enough horizontal space on the page. Add technical specificity, quantified context, \
or relevant detail to make the bullet longer — without inventing facts.\n\
\n\
Respond with valid JSON only: {\"text\": \"...\"}\n\
Do NOT use markdown code fences. Do NOT add any explanation outside the JSON object.";

pub const EXPAND_PROMPT_TEMPLATE: &str = "\
A resume bullet is too short and must be expanded to fill more of the line.\n\
\n\
CURRENT BULLET: {bullet_text}\n\
CURRENT FILL: {fill_percent}% of the line (minimum required: {required_percent}%)\n\
CHARACTER BUDGET: approximately {char_budget} characters for the expanded bullet\n\
JD KEYWORDS TO PRIORITIZE: {jd_keywords}\n\
\n\
EXPANSION RULES:\n\
1. Add technical specificity — exact tools, framework versions, infrastructure scale, or environment\n\
2. Add quantified context if currently missing (team size, duration, request volume, data scale)\n\
3. DO NOT invent metrics or outcomes not implied by the original bullet\n\
4. DO NOT change the contribution type or the opening action verb\n\
5. The result MUST be 1 or 2 printed lines MAXIMUM\n\
6. Prioritize the JD keywords listed above when adding detail\n\
\n\
Return JSON only: {\"text\": \"expanded bullet text here\"}";

// ────────────────────────────────────────────────────────────────────────────
// Compress prompt (bullet too long — 3+ lines)
// ────────────────────────────────────────────────────────────────────────────

pub const COMPRESS_SYSTEM: &str = "\
You are a resume bullet editor. Your task is to compress a resume bullet that exceeds \
2 printed lines. Remove redundant words and soft qualifiers — preserve quantified \
outcomes above all else.\n\
\n\
Respond with valid JSON only: {\"text\": \"...\"}\n\
Do NOT use markdown code fences. Do NOT add any explanation outside the JSON object.";

pub const COMPRESS_PROMPT_TEMPLATE: &str = "\
A resume bullet is too long and must be compressed to fit within 2 printed lines.\n\
\n\
CURRENT BULLET: {bullet_text}\n\
CURRENT LINES: {actual_lines} printed lines (maximum allowed: 2)\n\
CHARACTER BUDGET: approximately {char_budget} characters for the compressed bullet\n\
JD KEYWORDS TO PRESERVE: {jd_keywords}\n\
\n\
PRIORITY ORDER (keep > remove):\n\
1. KEEP: Quantified outcomes (%, $, x multipliers, counts, time reductions)\n\
2. KEEP: The primary technical claim and action verb\n\
3. KEEP: JD keywords listed above\n\
4. REMOVE: Redundant context phrases (\"in order to\", \"as a result of\")\n\
5. REMOVE: Soft qualifiers (\"various\", \"multiple\", \"significant\")\n\
6. REMOVE: Verbose prepositions and filler clauses\n\
\n\
The result MUST fit within 2 printed lines. Return JSON only: {\"text\": \"compressed bullet text here\"}";
