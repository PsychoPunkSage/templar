// Context Engine LLM prompt templates.
// All prompts for the context module are defined here.

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
