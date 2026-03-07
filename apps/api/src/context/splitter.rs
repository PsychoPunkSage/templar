//! Entry splitter: splits a raw multi-entry document into individual entry strings.
//!
//! Fast path: split on `\n---` horizontal rule lines.
//! Slow path: LLM pre-pass to identify natural section boundaries when no `---` found.
//!
//! The splitter enforces a hard cap of MAX_ENTRIES_PER_BATCH (50) entries per call.

use serde::Deserialize;

use crate::context::prompts::{SMART_SPLIT_PROMPT, SMART_SPLIT_SYSTEM};
use crate::errors::AppError;
use crate::llm_client::LlmClient;

const MAX_ENTRIES_PER_BATCH: usize = 50;
const MIN_ENTRY_LENGTH: usize = 20;

/// Split a multi-entry document on bare `---` horizontal rule lines.
///
/// Each `---` appearing after a newline (with optional surrounding whitespace)
/// is treated as a separator. Leading/trailing whitespace and `---` fragments
/// are trimmed from each segment. Segments shorter than 20 characters are filtered
/// out (handles lone `---` lines that appear as empty after splitting).
///
/// Returns `AppError::Validation` if:
/// - No valid entries are found (empty input or all-separators)
/// - Entry count exceeds MAX_ENTRIES_PER_BATCH (50)
///
/// Single-entry documents (no `---`) are valid and return a one-element Vec.
pub fn split_entries(raw: &str) -> Result<Vec<String>, AppError> {
    let entries: Vec<String> = raw
        .split("\n---")
        .map(|s| {
            // Trim leading/trailing whitespace AND any leading dashes
            // (handles the trailing fragment after the last `---`)
            s.trim_matches(|c: char| c == '\n' || c == '\r' || c == ' ')
                .trim_start_matches('-')
                .trim()
                .to_string()
        })
        .filter(|s| s.len() >= MIN_ENTRY_LENGTH)
        .collect();

    if entries.is_empty() {
        return Err(AppError::Validation(
            "No content found to ingest. Please provide at least one entry with meaningful text."
                .into(),
        ));
    }

    if entries.len() > MAX_ENTRIES_PER_BATCH {
        return Err(AppError::Validation(format!(
            "Document contains {} entries; maximum per batch is {}. \
             Please split your document into smaller uploads.",
            entries.len(),
            MAX_ENTRIES_PER_BATCH
        )));
    }

    Ok(entries)
}

// ────────────────────────────────────────────────────────────────────────────
// Phase 5.5.3 — Smart split with LLM fallback
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct SplitEntry {
    #[allow(dead_code)]
    entry_type: String,
    text: String,
}

/// Splits a document into individual entry strings.
///
/// **Fast path** (no LLM): if the document contains `\n---`, use the existing
/// `split_entries()` separator-based approach.
///
/// **Slow path** (LLM pre-pass): if no `---` separator is found, call the LLM
/// to identify natural section boundaries (one entry per company/project/skill).
/// Falls back to treating the whole document as a single entry if LLM fails.
pub async fn smart_split(raw_text: &str, llm: &LlmClient) -> Result<Vec<String>, AppError> {
    // Fast path: existing separator-based split
    if raw_text.contains("\n---") {
        return split_entries(raw_text);
    }

    // Short document (<200 chars) — treat as single entry without LLM overhead
    if raw_text.trim().len() < 200 {
        return split_entries(raw_text);
    }

    // Slow path: LLM identifies section boundaries
    let prompt = SMART_SPLIT_PROMPT.replace("{raw_text}", raw_text);
    let splits: Result<Vec<SplitEntry>, _> = llm.call_json(&prompt, SMART_SPLIT_SYSTEM).await;

    match splits {
        Ok(entries) => {
            let texts: Vec<String> = entries
                .into_iter()
                .map(|e| e.text.trim().to_string())
                .filter(|t| t.len() >= MIN_ENTRY_LENGTH)
                .collect();

            if texts.is_empty() {
                // LLM returned nothing useful — fall back to whole document
                return single_entry_fallback(raw_text);
            }
            if texts.len() > MAX_ENTRIES_PER_BATCH {
                return Err(AppError::Validation(format!(
                    "Document contains {} entries; maximum per batch is {}. \
                     Please split your document into smaller uploads.",
                    texts.len(),
                    MAX_ENTRIES_PER_BATCH
                )));
            }
            Ok(texts)
        }
        Err(e) => {
            // LLM failed — fall back to treating the whole document as one entry
            tracing::warn!(error = %e, "smart_split LLM call failed, treating as single entry");
            single_entry_fallback(raw_text)
        }
    }
}

fn single_entry_fallback(raw_text: &str) -> Result<Vec<String>, AppError> {
    let trimmed = raw_text.trim().to_string();
    if trimmed.len() < MIN_ENTRY_LENGTH {
        return Err(AppError::Validation(
            "No content found to ingest. Please provide at least one entry with meaningful text."
                .into(),
        ));
    }
    Ok(vec![trimmed])
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_entry_no_separator() {
        let raw = "This is my work experience at ACME Corp from 2020 to 2023.";
        let entries = split_entries(raw).unwrap();
        assert_eq!(entries.len(), 1);
        assert!(entries[0].contains("ACME Corp"));
    }

    #[test]
    fn test_two_entries_separated_by_triple_dash() {
        let raw = "First entry about working at Company A from 2020.\n---\nSecond entry about a project I built in 2022.";
        let entries = split_entries(raw).unwrap();
        assert_eq!(entries.len(), 2);
        assert!(entries[0].contains("Company A"));
        assert!(entries[1].contains("project"));
    }

    #[test]
    fn test_three_entries() {
        let raw = "Entry one: experience at Corp X in 2019.\n---\nEntry two: built distributed system.\n---\nEntry three: led team of five engineers.";
        let entries = split_entries(raw).unwrap();
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_leading_separator_is_ignored() {
        // Document starting with --- should produce clean entries
        let raw = "---\nFirst real entry about distributed systems work.\n---\nSecond entry about machine learning project.";
        let entries = split_entries(raw).unwrap();
        // The leading empty segment is filtered out
        assert!(entries.len() >= 1);
        assert!(entries.iter().all(|e| e.len() >= MIN_ENTRY_LENGTH));
    }

    #[test]
    fn test_trailing_separator_is_ignored() {
        let raw = "First entry about software engineering work at ACME.\n---";
        let entries = split_entries(raw).unwrap();
        assert_eq!(entries.len(), 1);
    }

    #[test]
    fn test_consecutive_separators_filtered() {
        let raw = "Good entry with real content about my work experience.\n---\n---\nAnother good entry about engineering projects.";
        let entries = split_entries(raw).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_empty_input_returns_error() {
        let result = split_entries("");
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_only_separators_returns_error() {
        let result = split_entries("\n---\n---\n---");
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_exceeds_max_entries_returns_error() {
        // Build a doc with 51 entries
        let entries: Vec<String> = (0..51)
            .map(|i| format!("Entry {i}: worked on project and delivered measurable results"))
            .collect();
        let raw = entries.join("\n---\n");
        let result = split_entries(&raw);
        assert!(matches!(result, Err(AppError::Validation(_))));
        if let Err(AppError::Validation(msg)) = result {
            assert!(msg.contains("51"));
            assert!(msg.contains("50"));
        }
    }

    #[test]
    fn test_exactly_max_entries_succeeds() {
        let entries: Vec<String> = (0..50)
            .map(|i| format!("Entry {i}: worked on project and delivered measurable results"))
            .collect();
        let raw = entries.join("\n---\n");
        let result = split_entries(&raw);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 50);
    }

    #[test]
    fn test_whitespace_trimmed_from_segments() {
        let raw = "  \n  Entry one with some real content at a company.  \n---\n  Entry two about another role.  \n";
        let entries = split_entries(raw).unwrap();
        assert_eq!(entries.len(), 2);
        // No leading/trailing whitespace in output
        for entry in &entries {
            assert_eq!(entry.trim(), entry.as_str());
        }
    }
}
