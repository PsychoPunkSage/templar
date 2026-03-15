#![allow(dead_code)]
//! Parameterized LaTeX templates for the five Templar resume styles.
//!
//! Each template maps to a FontFamily and differs in preamble constants:
//! spacing, rule thickness, section header style, and itemize settings.
//!
//! All 5 templates share the same document skeleton (geometry, fontspec,
//! microtype, enumitem, titlesec, xcolor, tabularx, parskip, hyperref).
//!
//! `escape_latex` is a single-pass character scanner — never use chained
//! `.replace()` which would double-escape backslashes.

use crate::layout::FontFamily;
use crate::render::types::{RenderParams, ResumeSection};

// ────────────────────────────────────────────────────────────────────────────
// Font / template metadata
// ────────────────────────────────────────────────────────────────────────────

/// Human-readable template name for a given font family.
pub fn template_name(font: &FontFamily) -> &'static str {
    match font {
        FontFamily::Inter => "Hacker",
        FontFamily::EbGaramond => "Researcher",
        FontFamily::Lato => "Operator",
        FontFamily::Oswald => "Founder",
        FontFamily::ComputerModern => "Classic",
    }
}

/// The fontspec package name used in `\setmainfont{...}`.
pub fn fontspec_name(font: &FontFamily) -> &'static str {
    match font {
        FontFamily::Inter => "Inter",
        FontFamily::EbGaramond => "EB Garamond",
        FontFamily::Lato => "Lato",
        // NOTE: Oswald is a variable-weight TTF distributed as a single file
        // (Oswald-Variable.ttf). xdvipdfmx cannot resolve "Bold" style from a
        // variable TTF by fc-name alone — it tries to locate a separate file for
        // the bold variant and fails with "Invalid TTC index (not TTC font)".
        // We use an explicit Path= declaration in the document preamble instead;
        // this value is used only for non-Oswald fonts in build_latex_document().
        // For Oswald the preamble injects the correct \setmainfont[Path=...] call.
        FontFamily::Oswald => "Oswald",
        FontFamily::ComputerModern => "Latin Modern Roman",
    }
}

/// Returns the `\setmainfont{...}` declaration for XeLaTeX.
///
/// Most fonts can be loaded by fc-name (e.g., `\setmainfont{Inter}`).
/// Oswald is a single variable-weight TTF file and requires an explicit
/// Path= declaration so xdvipdfmx can embed it correctly without attempting
/// to find a separate "Oswald Bold" file (which does not exist).
pub fn setmainfont_declaration(font: &FontFamily) -> String {
    match font {
        FontFamily::Oswald => {
            // Explicit path prevents xdvipdfmx "Invalid TTC index" error on variable TTFs.
            // UprightFont and BoldFont both point to the same variable TTF file —
            // the weight axis handles boldness at the OpenType level.
            r"[Path=/usr/local/share/fonts/templar/,Extension=.ttf,UprightFont=Oswald-Variable,BoldFont=Oswald-Variable]"
                .to_string()
        }
        other => format!("{{{}}}", fontspec_name(other)),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Section ordering
// ────────────────────────────────────────────────────────────────────────────

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
// Per-template preamble constants
// ────────────────────────────────────────────────────────────────────────────

fn template_preamble(font: &FontFamily) -> &'static str {
    match font {
        FontFamily::Inter => {
            // Hacker — minimal, dense, clean sans-serif headers.
            //
            // IMPORTANT: \sffamily in XeLaTeX+fontspec triggers a fallback to
            // Latin Modern Sans (lmss) TFM files when no \setsansfont is declared.
            // lmss is NOT in the Tectonic pre-warmed cache, causing a "font not
            // loadable" error. Fix: declare \setsansfont{Inter} so \sffamily stays
            // within the Inter font family (Inter is already sans-serif; this is a
            // no-op visually but prevents the lmsans fallback at the XeLaTeX level).
            r#"% Hacker template — clean, dense, minimal
\setlength{\parskip}{2pt}
\setsansfont{Inter}
\titleformat{\section}{\large\bfseries\sffamily}{}{0em}{}[\titlerule]
\titlespacing{\section}{0pt}{6pt}{4pt}"#
        }
        FontFamily::EbGaramond => {
            // Researcher — formal, publication-ready, small-caps headers
            r#"% Researcher template — formal, publication-ready
\setlength{\parskip}{4pt}
\titleformat{\section}{\normalsize\bfseries\scshape}{}{0em}{}[\titlerule]
\titlespacing{\section}{0pt}{8pt}{4pt}"#
        }
        FontFamily::Lato => {
            // Operator — spacious, achievement-forward, gray rule under headers
            r#"% Operator template — spacious, achievement-forward
\setlength{\parskip}{4pt}
\definecolor{rulecolor}{gray}{0.4}
\titleformat{\section}{\large\bfseries}{}{0em}{}[\color{rulecolor}\titlerule]
\titlespacing{\section}{0pt}{8pt}{5pt}"#
        }
        FontFamily::Oswald => {
            // Founder — bold condensed headers, startup-facing.
            //
            // IMPORTANT: \sffamily is removed here. Adding \sffamily with Oswald as
            // the main font (a variable TTF) causes XeLaTeX to fall back to lmss for
            // the sans-serif family, which is not cached. Since Oswald is already a
            // condensed sans-serif face, \sffamily is redundant and visually identical.
            // The Path= font declaration in build_latex_document() handles bold loading.
            r#"% Founder template — bold headers, startup-facing
\setlength{\parskip}{3pt}
\titleformat{\section}{\Large\bfseries}{}{0em}{}
\titlespacing{\section}{0pt}{6pt}{3pt}"#
        }
        FontFamily::ComputerModern => {
            // Classic — ATS-safe, plain, highly parseable
            r#"% Classic template — ATS-safe, plain, highly parseable
\setlength{\parskip}{2pt}
\titleformat{\section}{\normalsize\bfseries}{}{0em}{}[\hrule]
\titlespacing{\section}{0pt}{6pt}{4pt}"#
        }
    }
}

/// itemize options string for `\begin{itemize}[<opts>]`.
fn itemize_settings(font: &FontFamily) -> &'static str {
    match font {
        FontFamily::Inter => "leftmargin=1.5em, itemsep=1pt, parsep=0pt, topsep=2pt",
        FontFamily::EbGaramond => "leftmargin=1.5em, itemsep=2pt, parsep=0pt, topsep=3pt",
        FontFamily::Lato => "leftmargin=1.5em, itemsep=3pt, parsep=0pt, topsep=4pt",
        FontFamily::Oswald => "leftmargin=1.5em, itemsep=2pt, parsep=0pt, topsep=3pt",
        FontFamily::ComputerModern => "leftmargin=1.5em, itemsep=1pt, parsep=0pt, topsep=2pt",
    }
}

// ────────────────────────────────────────────────────────────────────────────
// LaTeX special-character escaping
// ────────────────────────────────────────────────────────────────────────────

/// Escapes all 10 LaTeX special characters in a single pass.
///
/// Characters handled: `& % $ # _ { } ~ ^ \`
///
/// Single-pass implementation avoids double-escaping backslashes that would
/// occur with chained `.replace()` calls.
pub fn escape_latex(text: &str) -> String {
    let mut result = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        match c {
            // Backslash must come first and use a command form
            '\\' => result.push_str(r"\textbackslash{}"),
            '&' => result.push_str(r"\&"),
            '%' => result.push_str(r"\%"),
            '$' => result.push_str(r"\$"),
            '#' => result.push_str(r"\#"),
            '_' => result.push_str(r"\_"),
            '{' => result.push_str(r"\{"),
            '}' => result.push_str(r"\}"),
            '~' => result.push_str(r"\textasciitilde{}"),
            '^' => result.push_str(r"\textasciicircum{}"),
            c => result.push(c),
        }
    }
    result
}

// ────────────────────────────────────────────────────────────────────────────
// Document builder
// ────────────────────────────────────────────────────────────────────────────

/// Builds a complete LaTeX document string from the given render parameters.
///
/// Structure:
/// ```text
/// \documentclass[<pt>]{article}
/// \usepackage[<margins>]{geometry}
/// \usepackage{fontspec}
/// \setmainfont{<fontspec_name>}
/// \usepackage{microtype,enumitem,...}
/// <template_preamble>
/// \begin{document}
/// \pagestyle{empty}
/// % Header placeholder
/// <sections>
/// \end{document}
/// ```
pub fn build_latex_document(params: &RenderParams) -> String {
    let font_decl = setmainfont_declaration(&params.font);
    let preamble = template_preamble(&params.font);
    let item_opts = itemize_settings(&params.font);
    let ordered = order_sections(&params.sections);

    let mut body = String::new();
    for section in &ordered {
        if section.bullets.is_empty() {
            continue;
        }
        body.push_str(&format!("\\section*{{{}}}\n", escape_latex(&section.name)));
        body.push_str(&format!("\\begin{{itemize}}[{}]\n", item_opts));
        for bullet in &section.bullets {
            body.push_str(&format!("  \\item {}\n", escape_latex(bullet)));
        }
        body.push_str("\\end{itemize}\n\n");
    }

    format!(
        r#"\documentclass[{font_size_pt}pt]{{article}}
\usepackage[left={margin_left:.2}in, right={margin_right:.2}in, top=0.75in, bottom=0.75in]{{geometry}}
\usepackage{{fontspec}}
\setmainfont{font_decl}
\usepackage{{microtype,enumitem,titlesec,xcolor,tabularx,parskip}}
\usepackage[hidelinks]{{hyperref}}
{preamble}
\begin{{document}}
\pagestyle{{empty}}
% Header placeholder (Phase 6 will wire real user data)
{body}\end{{document}}
"#,
        font_size_pt = params.font_size_pt,
        margin_left = params.margin_left_in,
        margin_right = params.margin_right_in,
        font_decl = font_decl,
        preamble = preamble,
        body = body,
    )
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::FontFamily;

    fn make_params(font: FontFamily) -> RenderParams {
        RenderParams {
            resume_id: uuid::Uuid::new_v4(),
            font,
            font_size_pt: 11,
            margin_left_in: 1.0,
            margin_right_in: 1.0,
            sections: vec![ResumeSection {
                name: "Experience".to_string(),
                bullets: vec![
                    "Built distributed cache reducing p99 latency by 40%".to_string(),
                    "Led migration to Kubernetes saving $50k/year".to_string(),
                ],
            }],
        }
    }

    // ── escape_latex ──────────────────────────────────────────────────────

    #[test]
    fn test_escape_latex_ampersand() {
        assert_eq!(escape_latex("A & B"), r"A \& B");
    }

    #[test]
    fn test_escape_latex_percent() {
        assert_eq!(escape_latex("50%"), r"50\%");
    }

    #[test]
    fn test_escape_latex_dollar() {
        assert_eq!(escape_latex("$100"), r"\$100");
    }

    #[test]
    fn test_escape_latex_underscore() {
        assert_eq!(escape_latex("foo_bar"), r"foo\_bar");
    }

    #[test]
    fn test_escape_latex_backslash_single_pass() {
        // A backslash should become \textbackslash{}, NOT \\textbackslash{} (no double-escape)
        let result = escape_latex("a\\b");
        assert_eq!(result, r"a\textbackslash{}b");
        // Crucially, the output does NOT start with \\
        assert!(!result.contains("\\\\"));
    }

    #[test]
    fn test_escape_latex_clean_text_unchanged() {
        let text = "Architected distributed caching layer";
        assert_eq!(escape_latex(text), text);
    }

    // ── build_latex_document ─────────────────────────────────────────────

    #[test]
    fn test_build_latex_contains_font_name() {
        let params = make_params(FontFamily::Inter);
        let doc = build_latex_document(&params);
        assert!(
            doc.contains("Inter"),
            "document must reference Inter font name"
        );
    }

    #[test]
    fn test_build_latex_contains_geometry_margins() {
        let params = make_params(FontFamily::Inter);
        let doc = build_latex_document(&params);
        assert!(
            doc.contains("left=1.00in"),
            "document must include left margin"
        );
        assert!(
            doc.contains("right=1.00in"),
            "document must include right margin"
        );
    }

    #[test]
    fn test_build_latex_contains_section_name() {
        let params = make_params(FontFamily::Inter);
        let doc = build_latex_document(&params);
        assert!(
            doc.contains("Experience"),
            "document must include section name"
        );
    }

    #[test]
    fn test_build_latex_contains_escaped_bullet_text() {
        let mut params = make_params(FontFamily::Inter);
        params.sections[0].bullets = vec!["Saved $50k/year".to_string()];
        let doc = build_latex_document(&params);
        // The $ must be escaped in the output
        assert!(
            doc.contains(r"\$50k/year"),
            "bullet dollar sign must be escaped in LaTeX output"
        );
    }

    #[test]
    fn test_build_latex_all_five_fonts_no_panic() {
        for font in [
            FontFamily::Inter,
            FontFamily::EbGaramond,
            FontFamily::Lato,
            FontFamily::Oswald,
            FontFamily::ComputerModern,
        ] {
            let params = make_params(font);
            let doc = build_latex_document(&params);
            assert!(!doc.is_empty(), "document must not be empty for {font:?}");
        }
    }

    #[test]
    fn test_order_sections_experience_first() {
        let sections = vec![
            ResumeSection {
                name: "Skills".to_string(),
                bullets: vec!["Rust".to_string()],
            },
            ResumeSection {
                name: "Experience".to_string(),
                bullets: vec!["Did things".to_string()],
            },
        ];
        let ordered = order_sections(&sections);
        assert_eq!(
            ordered[0].name, "Experience",
            "Experience must sort before Skills"
        );
    }

    #[test]
    fn test_full_document_starts_with_documentclass() {
        let params = make_params(FontFamily::Inter);
        let doc = build_latex_document(&params);
        assert!(
            doc.trim_start().starts_with(r"\documentclass"),
            "LaTeX document must start with \\documentclass"
        );
    }

    #[test]
    fn test_full_document_ends_with_end_document() {
        let params = make_params(FontFamily::Inter);
        let doc = build_latex_document(&params);
        assert!(
            doc.trim_end().ends_with(r"\end{document}"),
            "LaTeX document must end with \\end{{document}}"
        );
    }

    #[test]
    fn test_template_name_and_fontspec_name_distinct_for_all_five() {
        for font in [
            FontFamily::Inter,
            FontFamily::EbGaramond,
            FontFamily::Lato,
            FontFamily::Oswald,
            FontFamily::ComputerModern,
        ] {
            let tname = template_name(&font);
            let fname = fontspec_name(&font);
            assert!(
                !tname.is_empty(),
                "template_name must be non-empty for {font:?}"
            );
            assert!(
                !fname.is_empty(),
                "fontspec_name must be non-empty for {font:?}"
            );
            // They should be different strings (template name is brand, fontspec is font name)
            assert_ne!(
                tname, fname,
                "template_name and fontspec_name should differ for {font:?}"
            );
        }
    }

    // ── setmainfont_declaration ───────────────────────────────────────────

    #[test]
    fn test_oswald_uses_path_declaration() {
        // Oswald is a variable TTF — xdvipdfmx needs explicit Path= to avoid
        // "Invalid TTC index" error. Verify the declaration contains Path=.
        let decl = setmainfont_declaration(&FontFamily::Oswald);
        assert!(
            decl.contains("Path="),
            "Oswald font declaration must use Path= to resolve the variable TTF"
        );
        assert!(
            decl.contains("Oswald-Variable"),
            "Oswald declaration must reference Oswald-Variable.ttf"
        );
    }

    #[test]
    fn test_inter_uses_simple_name_declaration() {
        let decl = setmainfont_declaration(&FontFamily::Inter);
        assert_eq!(
            decl, "{Inter}",
            "Inter should use simple brace-wrapped name"
        );
    }

    #[test]
    fn test_hacker_preamble_contains_setsansfont() {
        // The Hacker template must declare \setsansfont{Inter} to prevent
        // \sffamily in \titleformat from falling back to lmsans (not cached).
        let params = make_params(FontFamily::Inter);
        let doc = build_latex_document(&params);
        assert!(
            doc.contains(r"\setsansfont{Inter}"),
            "Hacker (Inter) document must contain \\setsansfont{{Inter}} to prevent lmss fallback"
        );
    }

    #[test]
    fn test_founder_preamble_no_sffamily() {
        // The Founder template must NOT use \sffamily — Oswald is already sans-serif
        // and \sffamily with a variable TTF main font triggers lmss fallback.
        let params = make_params(FontFamily::Oswald);
        let doc = build_latex_document(&params);
        assert!(
            !doc.contains(r"\sffamily"),
            "Founder (Oswald) document must not contain \\sffamily to avoid lmss fallback"
        );
    }

    #[test]
    fn test_oswald_document_contains_path_declaration() {
        let params = make_params(FontFamily::Oswald);
        let doc = build_latex_document(&params);
        assert!(
            doc.contains("Path=/usr/local/share/fonts/templar/"),
            "Oswald document must use explicit font path for xdvipdfmx compatibility"
        );
    }
}
