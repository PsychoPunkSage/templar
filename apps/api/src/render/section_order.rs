use crate::render::types::ResumeSection;

/// Canonical section order: Experience → Education → Projects → Skills →
/// Publications → Other.
///
/// Sections not in the priority list are appended in their original order.
pub fn order_sections(sections: &[ResumeSection]) -> Vec<&ResumeSection> {
    const PRIORITY: &[&str] = &[
        "experience",
        "education",
        "projects",
        "skills",
        "publications",
    ];

    let mut ordered: Vec<&ResumeSection> = Vec::with_capacity(sections.len());

    // First pass: emit in priority order
    for &name in PRIORITY {
        if let Some(s) = sections.iter().find(|s| s.name.eq_ignore_ascii_case(name)) {
            ordered.push(s);
        }
    }

    // Second pass: append anything not in the priority list
    for s in sections {
        let already = PRIORITY.iter().any(|&p| s.name.eq_ignore_ascii_case(p));
        if !already {
            ordered.push(s);
        }
    }

    ordered
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_section(name: &str) -> ResumeSection {
        ResumeSection {
            name: name.to_string(),
            bullets: vec!["bullet".to_string()],
        }
    }

    #[test]
    fn test_order_sections_experience_first() {
        let sections = vec![make_section("Skills"), make_section("Experience")];
        let ordered = order_sections(&sections);
        assert_eq!(
            ordered[0].name, "Experience",
            "Experience must sort before Skills"
        );
    }

    #[test]
    fn test_order_sections_full_priority() {
        let sections = vec![
            make_section("Publications"),
            make_section("Skills"),
            make_section("Projects"),
            make_section("Education"),
            make_section("Experience"),
        ];
        let ordered = order_sections(&sections);
        assert_eq!(ordered[0].name, "Experience");
        assert_eq!(ordered[1].name, "Education");
        assert_eq!(ordered[2].name, "Projects");
        assert_eq!(ordered[3].name, "Skills");
        assert_eq!(ordered[4].name, "Publications");
    }

    #[test]
    fn test_unknown_sections_appended_in_original_order() {
        // Sections: Certifications (unknown), Experience (priority 0),
        //           Hobbies (unknown), Education (priority 1)
        let sections = vec![
            make_section("Certifications"),
            make_section("Experience"),
            make_section("Hobbies"),
            make_section("Education"),
        ];
        let ordered = order_sections(&sections);
        // Priority sections come first
        assert_eq!(ordered[0].name, "Experience");
        assert_eq!(ordered[1].name, "Education");
        // Unknown sections follow in their original relative order
        assert_eq!(ordered[2].name, "Certifications");
        assert_eq!(ordered[3].name, "Hobbies");
    }

    #[test]
    fn test_order_sections_case_insensitive() {
        let sections = vec![make_section("SKILLS"), make_section("experience")];
        let ordered = order_sections(&sections);
        assert_eq!(ordered[0].name, "experience");
        assert_eq!(ordered[1].name, "SKILLS");
    }

    #[test]
    fn test_order_sections_empty_input() {
        let sections: Vec<ResumeSection> = vec![];
        let ordered = order_sections(&sections);
        assert!(ordered.is_empty());
    }
}
