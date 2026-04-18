//! Greedy entry-atomic paginator for CV mode.
//!
//! # Contract
//! - Entries are NEVER split across pages: all bullets for a single context entry land
//!   on the same page or the next page, never partially on one and the rest on another.
//! - Section headers repeat at the top of each new page (seen_sections cleared on page advance).
//! - Pages are filled greedily in order of the input bullet sequence.
//! - `max_pages` caps the output: if bullets overflow the last page, they are silently
//!   dropped (the generation content selector should not produce more than can fit).
//!
//! # Usage
//! Called from the simulator after `run_simulation_loop` for CV mode requests.
//! Returns a `PaginationResult` which the caller uses to update `page_number` on
//! each `SimulatedBullet` and set `page_count` on `SimulationResult`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::generation::generator::EntryGroup;
use crate::layout::simulator::SimulatedBullet;

// ────────────────────────────────────────────────────────────────────────────
// Output types
// ────────────────────────────────────────────────────────────────────────────

/// Outcome of the paginator — maps each bullet (by index) to its page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaginationResult {
    /// Total pages actually used (1 ≤ page_count ≤ max_pages).
    pub page_count: u8,
    /// Maps bullet vector index → page number (1-based).
    /// Bullets that could not fit within max_pages are absent from this map.
    pub page_assignments: HashMap<usize, u8>,
}

// ────────────────────────────────────────────────────────────────────────────
// Core algorithm
// ────────────────────────────────────────────────────────────────────────────

/// Distributes `SimulatedBullet`s across pages using greedy entry-atomic pagination.
///
/// `entry_groups` is used to look up how many bullets belong to each source_entry_id,
/// so we can check whether the full group fits on the current page before placing it.
/// If a group doesn't fit but would fit on the next page, we advance the page.
/// If it doesn't fit on any single page, we place it anyway to avoid infinite loops
/// (the group will be flagged by the page fill pass).
///
/// # Parameters
/// - `bullets` — ordered slice from `SimulationResult.bullets`
/// - `entry_groups` — typed entry groups from `build_entry_groups()`
/// - `lines_per_page` — from `PageConfig.usable_height_lines`
/// - `max_pages` — from `GenerateRequest.max_pages` (capped at 8 for safety)
pub fn paginate_bullets(
    bullets: &[SimulatedBullet],
    _entry_groups: &[EntryGroup],
    lines_per_page: u16,
    max_pages: u8,
) -> PaginationResult {
    let max_pages = max_pages.min(8).max(1);

    // Build a lookup: source_entry_id → lines needed for the whole group.
    // We measure the group size as the sum of verified_line_count for all non-header bullets
    // that share that source_entry_id in the current bullet slice.
    let mut entry_line_counts: HashMap<Uuid, u16> = HashMap::new();
    for bullet in bullets {
        if !bullet.text.is_empty() {
            // header placeholders (empty text) count as 0 lines for placement purposes
            *entry_line_counts
                .entry(bullet.source_entry_id)
                .or_insert(0) += bullet.verified_line_count as u16;
        }
    }

    // Build an ordered list of distinct entry IDs (preserving first-occurrence order).
    let mut ordered_entry_ids: Vec<Uuid> = Vec::new();
    {
        let mut seen = std::collections::HashSet::new();
        for bullet in bullets {
            if seen.insert(bullet.source_entry_id) {
                ordered_entry_ids.push(bullet.source_entry_id);
            }
        }
    }

    let mut page_assignments: HashMap<usize, u8> = HashMap::new();
    let mut current_page: u8 = 1;
    let mut lines_used_on_page: u16 = 0;
    // Track which sections have appeared on the current page — reset on page advance
    // so section headers are re-emitted at the top of each new page.
    let mut seen_sections_on_page: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Process entries in order; for each entry, place all its bullets atomically.
    for entry_id in &ordered_entry_ids {
        let entry_lines = *entry_line_counts.get(entry_id).unwrap_or(&0);
        // +1 line for section header if this section hasn't appeared on current page yet
        let header_line: u16 = if needs_section_header(entry_id, bullets, &seen_sections_on_page) {
            1
        } else {
            0
        };
        // +1 line for the entry sub-header (company/role/etc.) placeholder
        let sub_header_line: u16 = 1;
        let total_needed = entry_lines + header_line + sub_header_line;

        // Check if we need to advance the page.
        // Rule: if the group + headers won't fit on current page, AND we haven't maxed out
        // pages, advance. Exception: if the group itself exceeds one full page, place it anyway.
        let wont_fit = lines_used_on_page + total_needed > lines_per_page;
        let fits_alone = entry_lines <= lines_per_page;
        if wont_fit && fits_alone && current_page < max_pages {
            current_page += 1;
            lines_used_on_page = 0;
            seen_sections_on_page.clear();
        }

        // Place all bullets for this entry on current_page.
        let section_for_entry = entry_section(entry_id, bullets);
        seen_sections_on_page.insert(section_for_entry);

        for (idx, bullet) in bullets.iter().enumerate() {
            if bullet.source_entry_id == *entry_id {
                page_assignments.insert(idx, current_page);
            }
        }

        lines_used_on_page += total_needed;
    }

    let page_count = current_page;

    PaginationResult {
        page_count,
        page_assignments,
    }
}

/// Applies the `PaginationResult` back to the bullet slice (mutating `page_number`).
///
/// Bullets absent from `result.page_assignments` retain their existing `page_number` (1).
pub fn apply_pagination(bullets: &mut Vec<SimulatedBullet>, result: &PaginationResult) {
    for (idx, bullet) in bullets.iter_mut().enumerate() {
        if let Some(&page) = result.page_assignments.get(&idx) {
            bullet.page_number = page;
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ────────────────────────────────────────────────────────────────────────────

/// Returns the section name for the first bullet of `entry_id` in `bullets`.
fn entry_section(entry_id: &Uuid, bullets: &[SimulatedBullet]) -> String {
    bullets
        .iter()
        .find(|b| b.source_entry_id == *entry_id)
        .map(|b| b.section.clone())
        .unwrap_or_default()
}

/// Returns true if the section for this entry has NOT yet appeared on the current page,
/// meaning a section header line will be needed.
fn needs_section_header(
    entry_id: &Uuid,
    bullets: &[SimulatedBullet],
    seen_sections: &std::collections::HashSet<String>,
) -> bool {
    let section = entry_section(entry_id, bullets);
    !seen_sections.contains(&section)
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generation::generator::{EntryDisplayHeader, EntryGroup};
    use crate::layout::simulator::SimulatedBullet;

    fn make_bullet(entry_id: Uuid, section: &str, line_count: u8) -> SimulatedBullet {
        SimulatedBullet {
            text: "Some bullet text here".to_string(),
            source_entry_id: entry_id,
            section: section.to_string(),
            entry_header_latex: None,
            verified_line_count: line_count,
            jd_keywords_used: vec![],
            was_adjusted: false,
            flagged_for_review: false,
            page_number: 1,
        }
    }

    fn make_header_bullet(entry_id: Uuid, section: &str) -> SimulatedBullet {
        SimulatedBullet {
            text: String::new(), // header placeholder
            source_entry_id: entry_id,
            section: section.to_string(),
            entry_header_latex: Some(r"\job{Acme}{Eng}{2020--2022}".to_string()),
            verified_line_count: 0,
            jd_keywords_used: vec![],
            was_adjusted: false,
            flagged_for_review: false,
            page_number: 1,
        }
    }

    fn make_group(entry_id: Uuid, section: &str, bullets: Vec<SimulatedBullet>) -> EntryGroup {
        EntryGroup {
            source_entry_id: entry_id,
            section: section.to_string(),
            display_header: EntryDisplayHeader::Other {
                label: "Test Entry".to_string(),
            },
            entry_header_latex: None,
            bullets,
        }
    }

    #[test]
    fn test_single_entry_fits_on_one_page() {
        let id = Uuid::new_v4();
        let bullets = vec![
            make_header_bullet(id, "experience"),
            make_bullet(id, "experience", 1),
            make_bullet(id, "experience", 2),
        ];
        let groups = vec![make_group(id, "experience", vec![bullets[1].clone()])];
        let result = paginate_bullets(&bullets, &groups, 45, 4);
        assert_eq!(result.page_count, 1);
        for idx in 0..3 {
            assert_eq!(result.page_assignments.get(&idx), Some(&1));
        }
    }

    #[test]
    fn test_two_entries_fit_on_one_page() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let bullets = vec![
            make_bullet(id1, "experience", 1),
            make_bullet(id1, "experience", 1),
            make_bullet(id2, "experience", 1),
            make_bullet(id2, "experience", 2),
        ];
        let groups = vec![
            make_group(id1, "experience", vec![bullets[0].clone(), bullets[1].clone()]),
            make_group(id2, "experience", vec![bullets[2].clone(), bullets[3].clone()]),
        ];
        let result = paginate_bullets(&bullets, &groups, 45, 4);
        assert_eq!(result.page_count, 1, "both entries should fit on page 1");
    }

    #[test]
    fn test_overflow_entry_moves_to_page_2() {
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        // Page has 10 lines. id1 fills 9 lines (entry_lines=7, +1 section header, +1 sub-header).
        // id2 needs 4 lines (entry_lines=2, +1 section header, +1 sub-header) → overflows to page 2.
        let mut bullets = Vec::new();
        for _ in 0..7 {
            bullets.push(make_bullet(id1, "experience", 1));
        }
        for _ in 0..2 {
            bullets.push(make_bullet(id2, "projects", 1));
        }

        let groups = vec![
            make_group(id1, "experience", bullets[0..7].to_vec()),
            make_group(id2, "projects", bullets[7..9].to_vec()),
        ];

        let result = paginate_bullets(&bullets, &groups, 10, 4);
        // id1 on page 1, id2 overflows to page 2
        for idx in 0..7 {
            assert_eq!(
                result.page_assignments.get(&idx),
                Some(&1),
                "id1 bullet {idx} should be on page 1"
            );
        }
        for idx in 7..9 {
            assert_eq!(
                result.page_assignments.get(&idx),
                Some(&2),
                "id2 bullet {idx} should be on page 2"
            );
        }
        assert_eq!(result.page_count, 2);
    }

    #[test]
    fn test_max_pages_cap() {
        // max_pages=1 — everything lands on page 1 even if overflowing.
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        let bullets = vec![
            make_bullet(id1, "experience", 5), // 5+1+1=7 of 6 available
            make_bullet(id2, "experience", 5), // would overflow but max_pages=1
        ];
        let groups = vec![
            make_group(id1, "experience", vec![bullets[0].clone()]),
            make_group(id2, "experience", vec![bullets[1].clone()]),
        ];
        let result = paginate_bullets(&bullets, &groups, 6, 1);
        assert_eq!(result.page_count, 1, "capped at max_pages=1");
    }

    #[test]
    fn test_max_pages_clamp_at_8() {
        // Requesting 100 pages should clamp to 8
        let id = Uuid::new_v4();
        let bullets = vec![make_bullet(id, "experience", 1)];
        let groups = vec![make_group(id, "experience", vec![bullets[0].clone()])];
        let result = paginate_bullets(&bullets, &groups, 45, 100);
        // Page count won't be 100; should be clamped
        assert!(result.page_count <= 8);
    }

    #[test]
    fn test_empty_bullets_returns_page_1() {
        let result = paginate_bullets(&[], &[], 45, 4);
        assert_eq!(result.page_count, 1);
        assert!(result.page_assignments.is_empty());
    }

    #[test]
    fn test_apply_pagination_updates_page_number() {
        let id = Uuid::new_v4();
        let mut bullets = vec![
            make_bullet(id, "experience", 1),
            make_bullet(id, "experience", 1),
        ];
        let mut assignments = HashMap::new();
        assignments.insert(0, 1u8);
        assignments.insert(1, 2u8);
        let result = PaginationResult {
            page_count: 2,
            page_assignments: assignments,
        };
        apply_pagination(&mut bullets, &result);
        assert_eq!(bullets[0].page_number, 1);
        assert_eq!(bullets[1].page_number, 2);
    }

    #[test]
    fn test_section_header_not_repeated_within_page() {
        // Two entries in the same section on the same page —
        // second entry should NOT count an extra section header line.
        let id1 = Uuid::new_v4();
        let id2 = Uuid::new_v4();
        // Page has exactly 10 lines.
        // id1: 4 entry lines + 1 section header + 1 sub-header = 6 lines
        // id2: 2 entry lines + 0 section header (same section already seen) + 1 sub-header = 3 lines
        // Total: 9 lines — both should fit on page 1.
        let bullets = vec![
            make_bullet(id1, "experience", 1),
            make_bullet(id1, "experience", 1),
            make_bullet(id1, "experience", 1),
            make_bullet(id1, "experience", 1),
            make_bullet(id2, "experience", 1),
            make_bullet(id2, "experience", 1),
        ];
        let groups = vec![
            make_group(id1, "experience", bullets[0..4].to_vec()),
            make_group(id2, "experience", bullets[4..6].to_vec()),
        ];
        let result = paginate_bullets(&bullets, &groups, 10, 4);
        // Both entries should be on page 1 (6+3=9 ≤ 10)
        for idx in 0..6 {
            assert_eq!(
                result.page_assignments.get(&idx),
                Some(&1),
                "bullet {idx} should be on page 1"
            );
        }
        assert_eq!(result.page_count, 1);
    }

    #[test]
    fn test_pagination_result_is_serializable() {
        let result = PaginationResult {
            page_count: 2,
            page_assignments: HashMap::from([(0, 1u8), (1, 2u8)]),
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: PaginationResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.page_count, 2);
    }
}
