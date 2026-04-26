use std::collections::HashMap;

use crate::models::context::ContextEntryRow;
use crate::models::resume::PersonaRow;

pub const PERSONA_SUGGEST_SYSTEM: &str = "\
You are a career intelligence assistant for a resume platform. \
You analyze a user's professional context to suggest resume personas — \
named viewpoints that emphasize different aspects of the user's career. \
You MUST respond with a valid JSON array only. \
Do NOT include any text outside the JSON array. \
Do NOT use markdown code fences. \
If the data is insufficient, return an empty array: []";

pub fn build_persona_suggest_prompt(
    entries: &[ContextEntryRow],
    existing: &[PersonaRow],
    top_tags: usize,
    top_entries: usize,
    max_count: usize,
) -> String {
    // Tag frequency map across all entries
    let mut tag_freq: HashMap<&str, usize> = HashMap::new();
    for entry in entries {
        for tag in &entry.tags {
            *tag_freq.entry(tag.as_str()).or_insert(0) += 1;
        }
    }

    // Top N tags by frequency (descending)
    let mut tag_vec: Vec<(&&str, &usize)> = tag_freq.iter().collect();
    tag_vec.sort_by(|a, b| b.1.cmp(a.1));
    let top_tags_list: Vec<String> = tag_vec
        .iter()
        .take(top_tags)
        .map(|(tag, count)| format!("{} ({})", tag, count))
        .collect();

    // Entry type distribution
    let mut type_dist: HashMap<&str, usize> = HashMap::new();
    for entry in entries {
        *type_dist.entry(entry.entry_type.as_str()).or_insert(0) += 1;
    }
    let mut type_vec: Vec<(&str, usize)> = type_dist.into_iter().collect();
    type_vec.sort_by(|a, b| b.1.cmp(&a.1));
    let type_summary: Vec<String> = type_vec
        .iter()
        .map(|(t, c)| format!("{}: {}", t, c))
        .collect();

    // Top N entries by recency * impact score — label only, no bullet text
    let mut scored: Vec<(f64, String)> = entries
        .iter()
        .map(|e| {
            let score = e.recency_score * e.impact_score;
            let label = e
                .data
                .get("role")
                .or_else(|| e.data.get("name"))
                .or_else(|| e.data.get("company"))
                .or_else(|| e.data.get("institution"))
                .and_then(|v| v.as_str())
                .unwrap_or("(unnamed)")
                .to_string();
            let tags_str = e.tags.join(", ");
            (
                score,
                format!("[{}] {} — tags: {}", e.entry_type, label, tags_str),
            )
        })
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let highlights: Vec<String> = scored
        .iter()
        .take(top_entries)
        .map(|(_, s)| s.clone())
        .collect();

    // Existing personas section — injected so the LLM avoids regenerating similar ones (Layer 1)
    let existing_section = if existing.is_empty() {
        String::new()
    } else {
        let lines: Vec<String> = existing
            .iter()
            .map(|p| {
                let tags = if p.emphasized_tags.is_empty() {
                    "(none)".to_string()
                } else {
                    p.emphasized_tags.join(", ")
                };
                format!("  - {} [emphasized: {}]", p.name, tags)
            })
            .collect();
        format!(
            "EXISTING PERSONAS (do NOT suggest semantically similar ones):\n{}\n\n",
            lines.join("\n")
        )
    };

    format!(
        r#"Analyze the following professional context and suggest 1-{max_count} distinct resume personas.

{existing_section}TAG FREQUENCY (tag: count):
{tags}

ENTRY TYPE DISTRIBUTION:
{types}

TOP ENTRIES BY RELEVANCE SCORE:
{entries_list}

Return a JSON array with 1-{max_count} persona objects. Each object MUST follow this EXACT schema:
[
  {{
    "name": "string — short persona label (e.g. 'ML Engineer', 'Startup PM')",
    "emphasized_tags": ["array of tag strings to boost — must come from the tag list above"],
    "suppressed_tags": ["array of tag strings to de-emphasize — must come from the tag list above"],
    "tone_preference": "one of: startup | enterprise | research | product | null",
    "reasoning": "1-2 sentence explanation of why this persona cluster makes sense"
  }}
]

RULES:
- Only suggest personas supported by the actual tags and entries provided.
- tone_preference MUST be exactly one of: "startup", "enterprise", "research", "product", or JSON null.
- emphasized_tags and suppressed_tags must only contain tags that appear in the TAG FREQUENCY list above.
- Do NOT invent tags that are not in the user's context.
- A suggestion is too similar to an existing persona if names overlap OR emphasized_tags Jaccard > 0.5.
- If the context is too sparse or homogeneous to form meaningful clusters, return [].
"#,
        max_count = max_count,
        existing_section = existing_section,
        tags = top_tags_list.join("\n"),
        types = type_summary.join(", "),
        entries_list = highlights.join("\n"),
    )
}
