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
            // En-dash / em-dash: replace with human-looking alternatives
            // (pdflatex can't render raw Unicode, and these read as AI-generated anyway)
            '\u{2013}' => result.push('-'), // en-dash → hyphen (ranges: 5-10ms)
            '\u{2014}' => result.push_str(", "), // em-dash → comma (sentence breaks)
            // Greek letters (math mode)
            '\u{03BC}' => result.push_str(r"$\mu$"),
            '\u{03B1}' => result.push_str(r"$\alpha$"),
            '\u{03B2}' => result.push_str(r"$\beta$"),
            '\u{03B3}' => result.push_str(r"$\gamma$"),
            '\u{03C0}' => result.push_str(r"$\pi$"),
            '\u{03C3}' => result.push_str(r"$\sigma$"),
            '\u{03A3}' => result.push_str(r"$\Sigma$"),
            '\u{03B4}' => result.push_str(r"$\delta$"),
            '\u{03A9}' => result.push_str(r"$\Omega$"),
            // Math symbols
            '\u{2265}' => result.push_str(r"$\geq$"),
            '\u{2264}' => result.push_str(r"$\leq$"),
            '\u{2260}' => result.push_str(r"$\neq$"),
            '\u{00D7}' => result.push_str(r"$\times$"),
            '\u{00F7}' => result.push_str(r"$\div$"),
            '\u{221E}' => result.push_str(r"$\infty$"),
            '\u{00B1}' => result.push_str(r"$\pm$"),
            // Arrows
            '\u{2192}' => result.push_str(r"$\rightarrow$"),
            '\u{2190}' => result.push_str(r"$\leftarrow$"),
            '\u{2194}' => result.push_str(r"$\leftrightarrow$"),
            '\u{21D2}' => result.push_str(r"$\Rightarrow$"),
            // Typography (textcomp commands)
            '\u{00B0}' => result.push_str(r"\textdegree{}"),
            '\u{00A9}' => result.push_str(r"\textcopyright{}"),
            '\u{00AE}' => result.push_str(r"\textregistered{}"),
            '\u{2122}' => result.push_str(r"\texttrademark{}"),
            '\u{00B7}' => result.push_str(r"\textperiodcentered{}"),
            '\u{2022}' => result.push_str(r"\textbullet{}"),
            // Smart quotes → proper LaTeX quotes
            '\u{2018}' => result.push_str(r"`"),  // ' → `
            '\u{2019}' => result.push_str(r"'"),  // ' → '
            '\u{201C}' => result.push_str(r"``"), // " → ``
            '\u{201D}' => result.push_str(r"''"), // " → ''
            // Ellipsis
            '\u{2026}' => result.push_str(r"\ldots{}"),
            // Non-breaking space → regular space
            '\u{00A0}' => result.push(' '),
            c => result.push(c),
        }
    }
    result
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_escape_latex_en_dash_to_hyphen() {
        assert_eq!(escape_latex("5\u{2013}10ms"), "5-10ms");
    }

    #[test]
    fn test_escape_latex_em_dash_to_comma() {
        assert_eq!(escape_latex("fast\u{2014}reliable"), "fast, reliable");
    }

    #[test]
    fn test_escape_latex_mu() {
        assert_eq!(escape_latex("latency: 5\u{03BC}s"), r"latency: 5$\mu$s");
    }

    #[test]
    fn test_escape_latex_smart_quotes() {
        assert_eq!(escape_latex("\u{201C}hello\u{201D}"), "``hello''");
        assert_eq!(escape_latex("\u{2018}it\u{2019}s"), "`it's");
    }

    #[test]
    fn test_escape_latex_arrows() {
        assert_eq!(escape_latex("A\u{2192}B"), r"A$\rightarrow$B");
    }

    #[test]
    fn test_escape_latex_ellipsis() {
        assert_eq!(escape_latex("and more\u{2026}"), r"and more\ldots{}");
    }
}
