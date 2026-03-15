use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::prompts::{DEDUP_CONFIRM_PROMPT, DEDUP_CONFIRM_SYSTEM};
use crate::llm_client::LlmClient;
use crate::models::context::ContextEntryRow;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictType {
    ContributionTypeMismatch,
    DateOverlap,
    DuplicateEntry,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictSeverity {
    Advisory,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictWarning {
    pub conflict_type: ConflictType,
    pub existing_entry_id: Uuid,
    pub description: String,
    pub severity: ConflictSeverity,
}

/// Checks for conflicts between a new entry and existing entries.
/// Returns advisory warnings (non-blocking).
pub fn check_for_conflicts(
    existing: &[ContextEntryRow],
    new_entry_type: &str,
    new_data: &serde_json::Value,
) -> Vec<ConflictWarning> {
    let mut warnings = Vec::new();

    if new_entry_type == "experience" {
        let new_company = new_data.get("company").and_then(|v| v.as_str());
        let new_role = new_data.get("role").and_then(|v| v.as_str());
        let new_ct = new_data.get("contribution_type").and_then(|v| v.as_str());
        let new_start = new_data.get("date_start").and_then(|v| v.as_str());
        let new_end = new_data.get("date_end").and_then(|v| v.as_str());

        for existing_entry in existing.iter().filter(|e| e.entry_type == "experience") {
            let ex_company = existing_entry.data.get("company").and_then(|v| v.as_str());
            let ex_role = existing_entry.data.get("role").and_then(|v| v.as_str());
            let ex_ct = existing_entry
                .data
                .get("contribution_type")
                .and_then(|v| v.as_str());
            let ex_start = existing_entry
                .data
                .get("date_start")
                .and_then(|v| v.as_str());
            let ex_end = existing_entry.data.get("date_end").and_then(|v| v.as_str());

            // Contribution type mismatch for same role
            if let (Some(nc), Some(nr), Some(nct), Some(ec), Some(er), Some(ect)) =
                (new_company, new_role, new_ct, ex_company, ex_role, ex_ct)
            {
                if nc.eq_ignore_ascii_case(ec) && nr.eq_ignore_ascii_case(er) && nct != ect {
                    warnings.push(ConflictWarning {
                        conflict_type: ConflictType::ContributionTypeMismatch,
                        existing_entry_id: existing_entry.entry_id,
                        description: format!(
                            "Existing entry at {} ({}) has contribution_type '{}', new has '{}'. Verify this is intentional.",
                            nc, nr, ect, nct
                        ),
                        severity: ConflictSeverity::Warning,
                    });
                }
            }

            // Date overlap at different companies
            if let (Some(ns), Some(ec), Some(es)) = (new_start, ex_company, ex_start) {
                if !new_company.unwrap_or("").eq_ignore_ascii_case(ec)
                    && dates_overlap(ns, new_end, es, ex_end)
                {
                    warnings.push(ConflictWarning {
                        conflict_type: ConflictType::DateOverlap,
                        existing_entry_id: existing_entry.entry_id,
                        description: format!(
                            "Date range overlaps with existing entry at '{}'. If simultaneous roles, this may be intentional.",
                            ec
                        ),
                        severity: ConflictSeverity::Advisory,
                    });
                }
            }
        }
    }

    warnings
}

fn dates_overlap(start1: &str, end1: Option<&str>, start2: &str, end2: Option<&str>) -> bool {
    let end1 = end1.unwrap_or("9999-12-31");
    let end2 = end2.unwrap_or("9999-12-31");
    start1 <= end2 && start2 <= end1
}

// ────────────────────────────────────────────────────────────────────────────
// Phase 5.5.4 — Dedup detection and merge decision
// ────────────────────────────────────────────────────────────────────────────

/// Result of a dedup check.
#[derive(Debug)]
pub enum DedupResult {
    /// No existing entry matches — proceed with normal insert.
    New,
    /// An existing entry matches — merge new data into it.
    Merged(Uuid),
    /// Dedup check failed — caller should fall back to normal insert.
    Error(String),
}

#[derive(Debug, Deserialize)]
struct DedupConfirmResponse {
    is_duplicate: bool,
    #[allow(dead_code)]
    confidence: f32,
    #[allow(dead_code)]
    reason: String,
}

/// Checks whether `new_entry` duplicates any existing entry and returns a `DedupResult`.
///
/// Algorithm:
/// 1. **Heuristic pre-filter**: match by company name / project name (case-insensitive).
///    Only entries with a name match are sent to the LLM.
/// 2. **LLM confirm**: for each candidate, ask the LLM "are these the same experience?".
///    Stop at the first confirmed duplicate.
/// 3. Return `DedupResult::Merged(existing_entry_id)` if a match is found, else `DedupResult::New`.
///
/// Errors are non-fatal — caller falls back to normal insert.
pub async fn check_and_merge(
    existing: &[ContextEntryRow],
    new_entry: &serde_json::Value,
    llm: &LlmClient,
) -> DedupResult {
    let candidates = heuristic_candidates(existing, new_entry);
    if candidates.is_empty() {
        return DedupResult::New;
    }

    let new_text = entry_to_text(new_entry);

    for candidate in candidates {
        let existing_text = entry_to_text(&serde_json::json!({
            "entry_type": candidate.entry_type,
            "data": candidate.data,
        }));

        let prompt = DEDUP_CONFIRM_PROMPT
            .replace("{existing_text}", &existing_text)
            .replace("{new_text}", &new_text);

        match llm
            .call_json::<DedupConfirmResponse>(&prompt, DEDUP_CONFIRM_SYSTEM)
            .await
        {
            Ok(resp) if resp.is_duplicate => {
                return DedupResult::Merged(candidate.entry_id);
            }
            Ok(_) => {
                // Not a duplicate — continue checking other candidates
            }
            Err(e) => {
                return DedupResult::Error(format!("LLM dedup confirm failed: {e}"));
            }
        }
    }

    DedupResult::New
}

/// Fast heuristic: find existing entries that share a company/project name with the new entry.
fn heuristic_candidates<'a>(
    existing: &'a [ContextEntryRow],
    new_entry: &serde_json::Value,
) -> Vec<&'a ContextEntryRow> {
    let new_data = new_entry.get("data").unwrap_or(new_entry);

    // Extract name-like fields from new entry
    let new_names: Vec<String> = [
        "company",
        "project_name",
        "institution",
        "organization",
        "name",
    ]
    .iter()
    .filter_map(|field| {
        new_data
            .get(field)
            .and_then(|v| v.as_str())
            .map(|s| s.to_lowercase())
    })
    .collect();

    if new_names.is_empty() {
        return vec![];
    }

    existing
        .iter()
        .filter(|e| {
            // Check if any name-like field in the existing entry matches
            for field in &[
                "company",
                "project_name",
                "institution",
                "organization",
                "name",
            ] {
                if let Some(ex_name) = e.data.get(field).and_then(|v| v.as_str()) {
                    let ex_name_lower = ex_name.to_lowercase();
                    if new_names.iter().any(|n| {
                        n == &ex_name_lower
                            || ex_name_lower.contains(n.as_str())
                            || n.contains(ex_name_lower.as_str())
                    }) {
                        return true;
                    }
                }
            }
            false
        })
        .collect()
}

/// Serialize an entry to a human-readable text for LLM comparison.
fn entry_to_text(entry: &serde_json::Value) -> String {
    serde_json::to_string_pretty(entry).unwrap_or_else(|_| entry.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_no_overlap_sequential() {
        assert!(!dates_overlap(
            "2020-01-01",
            Some("2020-12-31"),
            "2021-01-01",
            Some("2021-12-31")
        ));
    }

    #[test]
    fn test_overlap_partial() {
        assert!(dates_overlap(
            "2020-06-01",
            Some("2021-06-01"),
            "2021-01-01",
            Some("2022-01-01")
        ));
    }

    #[test]
    fn test_both_current() {
        assert!(dates_overlap("2022-01-01", None, "2021-06-01", None));
    }

    #[test]
    fn test_heuristic_candidates_same_company() {
        use chrono::Utc;
        use serde_json::json;

        let existing = vec![ContextEntryRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            version: 1,
            entry_type: "experience".to_string(),
            data: json!({"company": "Acme Corp", "role": "Engineer"}),
            raw_text: None,
            recency_score: 1.0,
            impact_score: 0.8,
            tags: vec![],
            flagged_evergreen: false,
            contribution_type: "primary_contributor".to_string(),
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: Utc::now(),
        }];

        let new_entry = json!({
            "entry_type": "experience",
            "data": {"company": "ACME Corp", "role": "Senior Engineer"}
        });

        let candidates = heuristic_candidates(&existing, &new_entry);
        assert_eq!(candidates.len(), 1);
    }

    #[test]
    fn test_heuristic_candidates_no_match() {
        use chrono::Utc;
        use serde_json::json;

        let existing = vec![ContextEntryRow {
            id: Uuid::new_v4(),
            user_id: Uuid::new_v4(),
            entry_id: Uuid::new_v4(),
            version: 1,
            entry_type: "experience".to_string(),
            data: json!({"company": "Acme Corp"}),
            raw_text: None,
            recency_score: 1.0,
            impact_score: 0.8,
            tags: vec![],
            flagged_evergreen: false,
            contribution_type: "primary_contributor".to_string(),
            quality_score: 1.0,
            quality_flags: vec![],
            created_at: Utc::now(),
        }];

        let new_entry = json!({
            "entry_type": "experience",
            "data": {"company": "Different Corp"}
        });

        let candidates = heuristic_candidates(&existing, &new_entry);
        assert!(candidates.is_empty());
    }
}
