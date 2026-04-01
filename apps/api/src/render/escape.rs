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
}
