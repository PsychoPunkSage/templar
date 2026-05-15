//! Cover letter generation pipeline.
//!
//! Reuses existing infrastructure:
//!   - `generation::jd_parser::parse_jd` — structured JD extraction
//!   - `context::versioning::get_current_entries` — user context loading
//!   - `generation::content_selector::select_content` — ranked entry selection
//!   - `generation::hash_utils::compute_jd_hash` — JD fingerprinting
//!   - `llm_client` — all LLM calls go through this module (CLAUDE.md rule)

use anyhow::Result;
use uuid::Uuid;

use crate::context::versioning::get_current_entries;
use crate::cover_letter::prompts::{COVER_LETTER_PROMPT_TEMPLATE, COVER_LETTER_SYSTEM};
use crate::cover_letter::{
    CoverLetterFocus, CoverLetterLlmOutput, CoverLetterParagraph, CoverLetterRow, CoverLetterTone,
    GenerateCoverLetterRequest,
};
use crate::generation::content_selector::select_content;
use crate::generation::generator::ResumeMode;
use crate::generation::hash_utils::compute_jd_hash;
use crate::generation::jd_parser::parse_jd;
use crate::models::resume::PersonaRow;
use crate::models::user::UserProfile;
use crate::state::AppState;

/// Maximum number of top-ranked context entries to include in the CL prompt.
const MAX_CONTEXT_ENTRIES: usize = 5;

/// Runs the cover letter generation pipeline synchronously.
///
/// Pipeline:
/// 1. Parse JD → structured keywords, requirements, tone
/// 2. Load user context entries
/// 3. Select + rank top-N entries via content_selector
/// 4. Build candidate summary from ranked entries
/// 5. Optionally load persona for tag emphasis
/// 6. Optionally load user profile for candidate name
/// 7. Build prompt and call LLM
/// 8. Persist to cover_letters table
pub async fn generate_cover_letter(
    state: &AppState,
    request: &GenerateCoverLetterRequest,
) -> Result<CoverLetterRow> {
    // Step 1: Parse JD
    let parsed_jd = parse_jd(&request.jd_text, &state.llm).await?;

    // Step 2: Load context entries
    let entries = get_current_entries(&state.db, request.user_id).await?;

    if entries.is_empty() {
        anyhow::bail!("No context entries found — add experience to generate a cover letter");
    }

    // Step 3: Select and rank entries
    let selection = select_content(
        entries,
        &parsed_jd,
        ResumeMode::SinglePage,
        None,
        &state.config,
    );

    // Step 4: Build candidate summary from top-N selected entries
    let top_entries: Vec<_> = selection
        .selected_entries
        .into_iter()
        .take(MAX_CONTEXT_ENTRIES)
        .collect();

    let candidate_summary = build_candidate_summary(&top_entries);

    // Step 5: Load persona if provided
    let persona: Option<PersonaRow> = if let Some(pid) = request.persona_id {
        sqlx::query_as::<_, PersonaRow>("SELECT * FROM personas WHERE id = $1")
            .bind(pid)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    // Step 6: Load user profile for candidate name
    let candidate_name = load_candidate_name(&state.db, request.user_id).await;

    // Step 7: Build prompt
    let jd_hash = compute_jd_hash(&request.jd_text);
    let prompt = build_prompt(
        &request.tone,
        &request.focus,
        persona.as_ref(),
        &candidate_name,
        &candidate_summary,
        &parsed_jd,
    );

    // Step 8: Call LLM
    let llm_output: CoverLetterLlmOutput = state
        .llm
        .call_json(&prompt, COVER_LETTER_SYSTEM)
        .await
        .map_err(|e| anyhow::anyhow!("Cover letter LLM call failed: {e}"))?;

    // Step 9: Convert to paragraphs
    let paragraphs = vec![
        CoverLetterParagraph {
            role: "hook".into(),
            text: llm_output.hook,
        },
        CoverLetterParagraph {
            role: "fit".into(),
            text: llm_output.fit,
        },
        CoverLetterParagraph {
            role: "culture".into(),
            text: llm_output.culture,
        },
        CoverLetterParagraph {
            role: "close".into(),
            text: llm_output.close,
        },
    ];
    let content_json = serde_json::to_value(&paragraphs)
        .map_err(|e| anyhow::anyhow!("Failed to serialize paragraphs: {e}"))?;

    // Step 10: Persist
    let row = sqlx::query_as::<_, CoverLetterRow>(
        r#"INSERT INTO cover_letters
              (user_id, resume_id, persona_id, jd_text_hash, tone, focus, content, company_name, role_title)
           VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
           RETURNING *"#,
    )
    .bind(request.user_id)
    .bind(request.resume_id)
    .bind(request.persona_id)
    .bind(&jd_hash)
    .bind(request.tone.as_str())
    .bind(request.focus.as_str())
    .bind(&content_json)
    .bind(llm_output.company_name.trim())
    .bind(llm_output.role_title.trim())
    .fetch_one(&state.db)
    .await?;

    Ok(row)
}

// ────────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────────

fn build_candidate_summary(entries: &[crate::generation::content_selector::RankedEntry]) -> String {
    entries
        .iter()
        .enumerate()
        .map(|(i, ranked)| {
            let entry = &ranked.entry;
            let data_preview =
                serde_json::to_string(&entry.data).unwrap_or_else(|_| "{}".to_string());
            let raw_preview = entry
                .raw_text
                .as_deref()
                .unwrap_or("")
                .chars()
                .take(400)
                .collect::<String>();
            format!(
                "[{i}] Type: {entry_type} | Score: {score:.2}\n\
                 Data: {data}\n\
                 Notes: {raw}",
                i = i + 1,
                entry_type = entry.entry_type,
                score = ranked.combined_score,
                data = data_preview,
                raw = raw_preview,
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn build_prompt(
    tone: &CoverLetterTone,
    focus: &CoverLetterFocus,
    persona: Option<&PersonaRow>,
    candidate_name: &str,
    candidate_summary: &str,
    parsed_jd: &crate::generation::jd_parser::ParsedJD,
) -> String {
    let persona_instruction = persona
        .map(|p| {
            let mut parts = Vec::new();
            if !p.emphasized_tags.is_empty() {
                parts.push(format!(
                    "Emphasize these areas: {}",
                    p.emphasized_tags.join(", ")
                ));
            }
            if !p.suppressed_tags.is_empty() {
                parts.push(format!("De-emphasize: {}", p.suppressed_tags.join(", ")));
            }
            if let Some(tp) = &p.tone_preference {
                parts.push(format!("Persona tone preference: {tp}"));
            }
            format!("PERSONA CONTEXT:\n{}", parts.join("\n"))
        })
        .unwrap_or_default();

    let jd_context = build_jd_context(parsed_jd);

    let keywords: Vec<String> = parsed_jd
        .keyword_inventory
        .iter()
        .take(10)
        .map(|k| format!("{} (weight: {:.1})", k.keyword, k.weighted_score))
        .collect();
    let keywords_str = keywords.join(", ");

    COVER_LETTER_PROMPT_TEMPLATE
        .replace("{tone_instruction}", tone.as_instruction())
        .replace("{focus_instruction}", focus.as_instruction())
        .replace("{persona_instruction}", &persona_instruction)
        .replace("{candidate_name}", candidate_name)
        .replace("{candidate_summary}", candidate_summary)
        .replace("{jd_context}", &jd_context)
        .replace("{keywords}", &keywords_str)
}

fn build_jd_context(parsed_jd: &crate::generation::jd_parser::ParsedJD) -> String {
    let seniority = &parsed_jd.role_signals.seniority;
    let is_startup = parsed_jd.role_signals.is_startup;
    let detected_tone = format!("{:?}", parsed_jd.detected_tone);

    let hard_reqs: Vec<String> = parsed_jd
        .hard_requirements
        .iter()
        .take(5)
        .map(|r| format!("  - {}", r.text))
        .collect();

    let soft_signals: Vec<String> = parsed_jd
        .soft_signals
        .iter()
        .take(3)
        .map(|s| format!("  - {s}"))
        .collect();

    format!(
        "Seniority: {seniority}\n\
         Culture: {culture}\n\
         Detected tone: {detected_tone}\n\
         Hard requirements:\n{hard_reqs}\n\
         Soft signals:\n{soft_signals}",
        seniority = seniority,
        culture = if is_startup {
            "startup / fast-paced"
        } else {
            "enterprise / structured"
        },
        detected_tone = detected_tone,
        hard_reqs = if hard_reqs.is_empty() {
            "  (none specified)".to_string()
        } else {
            hard_reqs.join("\n")
        },
        soft_signals = if soft_signals.is_empty() {
            "  (none)".to_string()
        } else {
            soft_signals.join("\n")
        },
    )
}

async fn load_candidate_name(db: &sqlx::PgPool, user_id: Uuid) -> String {
    sqlx::query_as::<_, UserProfile>("SELECT * FROM user_profiles WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
        .map(|p| p.full_name)
        .filter(|n| !n.trim().is_empty())
        .unwrap_or_else(|| "The Candidate".to_string())
}
