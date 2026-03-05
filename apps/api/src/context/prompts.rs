// Context Engine LLM prompt templates.
// All prompts for the context module are defined here.

// ────────────────────────────────────────────────────────────────────────────
// Phase 5.5.3 — Smart splitting
// ────────────────────────────────────────────────────────────────────────────

pub const SMART_SPLIT_SYSTEM: &str = "\
You are splitting a professional document into individual context entries. \
Identify each distinct experience, project, skill, or achievement. \
Group related bullets under the same entry (e.g. all bullets for one company = one entry). \
You MUST respond with valid JSON only — a JSON array of objects. \
Do NOT include any text outside the JSON array. \
Do NOT use markdown code fences.";

pub const SMART_SPLIT_PROMPT: &str = r#"Split the following professional document into individual context entries.

Return a JSON array where each element has this structure:
[
  {"entry_type": "experience", "text": "Full text of this specific entry..."},
  {"entry_type": "project",    "text": "Full text of this project entry..."}
]

Allowed entry_type values: experience | education | project | skill | publication | open_source | award | certification | extracurricular

Rules:
1. Each entry should describe ONE distinct experience, job, project, skill set, or achievement
2. Group all bullets/details for the same company or project into one entry
3. Do NOT split a single job role into multiple entries — keep them together
4. Skills can be grouped by category (e.g., one entry for "Programming Languages", one for "Tools")
5. If the document has only one experience, return a single-element array
6. Preserve all original text verbatim within each entry's "text" field

DOCUMENT:
{raw_text}"#;

// ────────────────────────────────────────────────────────────────────────────
// Phase 5.5.4 — Deduplication
// ────────────────────────────────────────────────────────────────────────────

pub const DEDUP_CONFIRM_SYSTEM: &str = "\
You are comparing two professional context entries to determine if they describe the same experience. \
You MUST respond with valid JSON only. \
Do NOT include any text outside the JSON object.";

pub const DEDUP_CONFIRM_PROMPT: &str = r#"Are these two professional context entries describing the same experience, job role, or project?

EXISTING ENTRY:
{existing_text}

NEW ENTRY:
{new_text}

Return JSON:
{"is_duplicate": true|false, "confidence": 0.0-1.0, "reason": "brief explanation"}

Consider them duplicates if they refer to the same company+role, same project name, or same achievement.
Be conservative: only return is_duplicate=true if you are at least 80% confident."#;

// ────────────────────────────────────────────────────────────────────────────
// Phase 5.5.4 — Merging
// ────────────────────────────────────────────────────────────────────────────

pub const MERGE_ENTRIES_SYSTEM: &str = "\
You are merging two descriptions of the same professional experience into one comprehensive entry. \
You MUST respond with valid JSON only — the same structure as the input entries. \
Do NOT include any text outside the JSON object. \
Do NOT use markdown code fences.";

pub const MERGE_ENTRIES_PROMPT: &str = r#"Merge these two descriptions of the same experience into one comprehensive entry.

EXISTING ENTRY:
{existing_json}

NEW ENTRY (additional details to merge in):
{new_json}

Rules:
1. Preserve ALL unique technical details, specific metrics, and achievements from both entries
2. Eliminate exact redundancy — do not repeat the same bullet twice
3. If both have bullets for the same accomplishment, keep the more specific/quantified version
4. Return the same JSON structure as the input (same entry_type and data schema)
5. The merged entry should be richer than either input alone

Return the merged entry as a single JSON object with the same schema as the inputs."#;

pub const CONTEXT_PARSE_SYSTEM: &str = "\
You are a precise resume data extractor. \
Parse natural language professional experience into structured JSON. \
You MUST respond with valid JSON only — no markdown fences, no explanations. \
Extract contribution_type HONESTLY — never inflate a team role to sole_author. \
If contribution_type is unclear from context, default to 'team_member'.";

pub const CONTEXT_PARSE_PROMPT: &str = r#"Parse the following professional experience text into a structured JSON object.

INPUT TEXT:
{raw_text}

OUTPUT SCHEMA (return exactly this structure):
{
  "entry_type": "experience" | "education" | "project" | "skill" | "publication" | "open_source" | "award" | "certification" | "extracurricular",
  "data": {
    // For "experience":
    // "company": "string", "role": "string", "date_start": "YYYY-MM-DD",
    // "date_end": "YYYY-MM-DD" | null (null = current),
    // "team_size": number | null, "tech_stack": ["string"],
    // "contribution_type": "sole_author" | "primary_contributor" | "team_member" | "reviewer",
    // "location": "string" | null,
    // "bullets": [{"text": "string", "impact_markers": ["string"], "confidence_marker": null | "[LOW_METRICS]"}]

    // For "education":
    // "institution": "string", "degree": "string", "field": "string",
    // "date_start": "YYYY-MM-DD", "date_end": "YYYY-MM-DD" | null,
    // "gpa": number | null, "honors": ["string"], "relevant_courses": ["string"]

    // For "project":
    // "name": "string", "description": "string", "tech_stack": ["string"],
    // "date_start": "YYYY-MM-DD" | null, "date_end": "YYYY-MM-DD" | null,
    // "url": "string" | null,
    // "contribution_type": "sole_author" | "primary_contributor" | "team_member" | "reviewer",
    // "bullets": [{"text": "string", "impact_markers": ["string"], "confidence_marker": null | "[LOW_METRICS]"}]

    // For "skill":
    // "category": "string", "items": ["string"],
    // "proficiency": "expert" | "proficient" | "familiar" | null

    // For "certification":
    // "name": "string", "issuer": "string", "date_issued": "YYYY-MM-DD",
    // "date_expires": "YYYY-MM-DD" | null, "credential_id": "string" | null

    // For "award":
    // "title": "string", "issuer": "string", "date": "YYYY-MM-DD", "description": "string" | null

    // For "publication":
    // "title": "string", "venue": "string", "date": "YYYY-MM-DD",
    // "authors": ["string"], "url": "string" | null,
    // "contribution_type": "sole_author" | "primary_contributor" | "team_member" | "reviewer"

    // For "open_source":
    // "project_name": "string", "description": "string", "url": "string" | null,
    // "contribution_type": "sole_author" | "primary_contributor" | "team_member" | "reviewer",
    // "tech_stack": ["string"],
    // "bullets": [{"text": "string", "impact_markers": ["string"], "confidence_marker": null | "[LOW_METRICS]"}]

    // For "extracurricular":
    // "organization": "string", "role": "string", "date_start": "YYYY-MM-DD",
    // "date_end": "YYYY-MM-DD" | null,
    // "bullets": [{"text": "string", "impact_markers": ["string"], "confidence_marker": null | "[LOW_METRICS]"}]
  }
}

RULES:
1. contribution_type must be honest: if they said "we" or "team", use "team_member"
2. Extract ALL numbers, percentages, times, and dollar amounts as impact_markers
3. If a bullet has no metrics, set confidence_marker to "[LOW_METRICS]"
4. Dates must be "YYYY-MM-DD". Use "YYYY-01-01" if only year is known.
5. Return ONLY the JSON object — nothing else, no code fences."#;
