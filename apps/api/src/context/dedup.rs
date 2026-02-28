use serde::{Deserialize, Serialize};
use uuid::Uuid;

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
}
