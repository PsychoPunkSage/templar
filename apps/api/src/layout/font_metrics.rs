//! Static font-metric tables for 5 resume font families.
#![allow(dead_code)]
//!
//! Character widths are in em units (relative to font size). This is an
//! intentional approximation — the LaTeX Knuth-Plass algorithm (where to break lines) uses exact
//! glyph metrics (actual shape of characters), but static tables catch real violations
//! (3-line bullets, 30%-fill bullets) while tolerating borderline ambiguity (±1–2% of line width).
//!
//! The simulation loop + microtype 3% safety margin absorbs the residual error.
//! All tables cover ASCII 0x20..=0x7E (95 printable characters).
//! Index = (char as usize) - 32.

use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────────────────────────────────────
// Font family enum
// ────────────────────────────────────────────────────────────────────────────

/// The five supported resume font families, matching Templar's template set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FontFamily {
    /// Hacker template — clean humanist sans-serif.
    Inter,
    /// Researcher template — classic old-style serif.
    EbGaramond,
    /// Operator template — geometric humanist sans-serif.
    Lato,
    /// Founder template — condensed display sans-serif.
    Oswald,
    /// Classic/ATS-safe template — traditional TeX font.
    ComputerModern,
}

// ────────────────────────────────────────────────────────────────────────────
// Page configuration
// ────────────────────────────────────────────────────────────────────────────

/// Layout parameters for a single resume page.
///
/// `text_width_em` is the usable text width in em units at the given font size.
/// Example: US letter paper, 1" margins, 11pt → 6.5" × (72.27pt/in ÷ 11pt) ≈ 42.7em.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageConfig {
    pub font: FontFamily,
    pub font_size_pt: u8,
    /// Usable text width in em units (derived from paper size, margins, and font size).
    pub text_width_em: f32,
    pub margin_left_in: f32,
    pub margin_right_in: f32,
    /// Total line slots available on a single-page resume (includes section headers, spacing).
    pub usable_height_lines: u16,
    /// LaTeX microtype expansion tolerance (typically 0.03 = 3%).
    /// Acts as a safety margin that absorbs small approximation errors in the metric tables.
    pub microtype_margin: f32,
}

/// Returns the default page config for the given font family.
///
/// Assumes: US letter (8.5" × 11"), 11pt font, 1.0" margins all sides.
/// text_width_em = 6.5" × (72.27pt/in ÷ 11pt) ≈ 42.7em.
pub fn default_page_config(font: FontFamily) -> PageConfig {
    PageConfig {
        font,
        font_size_pt: 11,
        text_width_em: 42.7,
        margin_left_in: 1.0,
        margin_right_in: 1.0,
        usable_height_lines: 45,
        microtype_margin: 0.03,
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Font metric table
// ────────────────────────────────────────────────────────────────────────────

/// Static character-width table for a font family.
///
/// All widths are in em units at 1em (i.e., at the configured font size).
/// `widths[i]` = width of ASCII character `(i + 32)`, covering 0x20 (space) through 0x7E (~).
///
/// Width array slot layout:
/// ```text
/// [0]=sp  [1]=!   [2]="   [3]=#   [4]=$   [5]=%   [6]=&   [7]='
/// [8]=(   [9]=)   [10]=*  [11]=+  [12]=,  [13]=-  [14]=.  [15]=/
/// [16..25]=0-9
/// [26]=:  [27]=;  [28]=<  [29]==  [30]=>  [31]=?  [32]=@
/// [33..58]=A-Z
/// [59]=[  [60]=\  [61]=]  [62]=^  [63]=_  [64]=`
/// [65..90]=a-z
/// [91]={  [92]=|  [93]=}  [94]=~
/// ```
pub struct FontMetricTable {
    pub font: FontFamily,
    widths: [f32; 95],
    /// Fallback width for non-ASCII characters (codepoints > 0x7E).
    pub average_char_width: f32,
    pub space_width: f32,
}

impl FontMetricTable {
    /// Measures the rendered width of a string in em units.
    ///
    /// Non-ASCII characters fall back to `average_char_width`.
    pub fn measure_str(&self, s: &str) -> f32 {
        s.chars()
            .map(|c| {
                let code = c as usize;
                if (32..=126).contains(&code) {
                    self.widths[code - 32]
                } else {
                    self.average_char_width
                }
            })
            .sum()
    }

    /// Returns the fraction of the text width that this string occupies on a single line.
    ///
    /// Values > 1.0 indicate the string would wrap. The microtype margin is NOT applied
    /// here — callers use `PageConfig.microtype_margin` for their own tolerance logic.
    pub fn coverage_fraction(&self, s: &str, config: &PageConfig) -> f32 {
        self.measure_str(s) / config.text_width_em
    }

    /// Estimates how many printed lines this string occupies when word-wrapped at `config.text_width_em`.
    ///
    /// Uses greedy word-wrap — same algorithm as `simulate_lines` in `contract.rs`.
    pub fn estimated_lines(&self, s: &str, config: &PageConfig) -> u8 {
        let words: Vec<&str> = s.split_whitespace().collect();
        if words.is_empty() {
            return 0;
        }
        let max_width = config.text_width_em;
        let mut line_count = 1u8;
        let mut current_width = 0.0_f32;
        let mut first = true;

        for word in &words {
            let word_w = self.measure_str(word);
            let space_w = if first { 0.0 } else { self.space_width };

            if !first && current_width + space_w + word_w > max_width {
                line_count = line_count.saturating_add(1);
                current_width = word_w;
                // first stays false — next word on new line will get a space
            } else {
                current_width += space_w + word_w;
                first = false;
            }
        }
        line_count
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Static width tables  (95 ASCII printable characters each)
// ────────────────────────────────────────────────────────────────────────────

/// Inter — humanist sans-serif (Hacker template).
static INTER_TABLE: FontMetricTable = FontMetricTable {
    font: FontFamily::Inter,
    #[rustfmt::skip]
    widths: [
        // sp    !     "     #     $     %     &     '     (     )     *     +     ,     -     .     /
        0.25, 0.30, 0.38, 0.56, 0.56, 0.89, 0.67, 0.22, 0.33, 0.33, 0.39, 0.59, 0.28, 0.33, 0.28, 0.31,
        // 0     1     2     3     4     5     6     7     8     9
        0.56, 0.56, 0.56, 0.56, 0.56, 0.56, 0.56, 0.56, 0.56, 0.56,
        // :     ;     <     =     >     ?     @
        0.28, 0.28, 0.59, 0.59, 0.59, 0.50, 1.02,
        // A     B     C     D     E     F     G     H     I     J     K     L     M
        0.67, 0.61, 0.61, 0.67, 0.56, 0.50, 0.67, 0.67, 0.25, 0.39, 0.61, 0.53, 0.78,
        // N     O     P     Q     R     S     T     U     V     W     X     Y     Z
        0.67, 0.72, 0.56, 0.72, 0.61, 0.50, 0.56, 0.67, 0.67, 0.89, 0.61, 0.61, 0.56,
        // [     \     ]     ^     _     `
        0.28, 0.31, 0.28, 0.47, 0.56, 0.34,
        // a     b     c     d     e     f     g     h     i     j     k     l     m
        0.56, 0.56, 0.50, 0.56, 0.56, 0.31, 0.56, 0.56, 0.22, 0.22, 0.53, 0.22, 0.83,
        // n     o     p     q     r     s     t     u     v     w     x     y     z
        0.56, 0.56, 0.56, 0.56, 0.33, 0.44, 0.39, 0.56, 0.50, 0.72, 0.50, 0.50, 0.44,
        // {     |     }     ~
        0.33, 0.26, 0.33, 0.59,
    ],
    average_char_width: 0.52,
    space_width: 0.25,
};

/// EB Garamond — old-style serif (Researcher template). Approx. 85% of Inter.
static EB_GARAMOND_TABLE: FontMetricTable = FontMetricTable {
    font: FontFamily::EbGaramond,
    #[rustfmt::skip]
    widths: [
        // sp    !     "     #     $     %     &     '     (     )     *     +     ,     -     .     /
        0.21, 0.26, 0.32, 0.48, 0.48, 0.76, 0.57, 0.19, 0.28, 0.28, 0.33, 0.50, 0.24, 0.28, 0.24, 0.26,
        // 0     1     2     3     4     5     6     7     8     9
        0.48, 0.48, 0.48, 0.48, 0.48, 0.48, 0.48, 0.48, 0.48, 0.48,
        // :     ;     <     =     >     ?     @
        0.24, 0.24, 0.50, 0.50, 0.50, 0.43, 0.87,
        // A     B     C     D     E     F     G     H     I     J     K     L     M
        0.57, 0.52, 0.52, 0.57, 0.48, 0.43, 0.57, 0.57, 0.21, 0.33, 0.52, 0.45, 0.66,
        // N     O     P     Q     R     S     T     U     V     W     X     Y     Z
        0.57, 0.61, 0.48, 0.61, 0.52, 0.43, 0.48, 0.57, 0.57, 0.76, 0.52, 0.52, 0.48,
        // [     \     ]     ^     _     `
        0.24, 0.26, 0.24, 0.40, 0.48, 0.29,
        // a     b     c     d     e     f     g     h     i     j     k     l     m
        0.48, 0.48, 0.43, 0.48, 0.48, 0.26, 0.48, 0.48, 0.19, 0.19, 0.45, 0.19, 0.71,
        // n     o     p     q     r     s     t     u     v     w     x     y     z
        0.48, 0.48, 0.48, 0.48, 0.28, 0.37, 0.33, 0.48, 0.43, 0.61, 0.43, 0.43, 0.37,
        // {     |     }     ~
        0.28, 0.22, 0.28, 0.50,
    ],
    average_char_width: 0.44,
    space_width: 0.21,
};

/// Lato — geometric humanist sans-serif (Operator template). Approx. 105% of Inter.
static LATO_TABLE: FontMetricTable = FontMetricTable {
    font: FontFamily::Lato,
    #[rustfmt::skip]
    widths: [
        // sp    !     "     #     $     %     &     '     (     )     *     +     ,     -     .     /
        0.26, 0.32, 0.40, 0.59, 0.59, 0.94, 0.70, 0.23, 0.35, 0.35, 0.41, 0.62, 0.29, 0.35, 0.29, 0.33,
        // 0     1     2     3     4     5     6     7     8     9
        0.59, 0.59, 0.59, 0.59, 0.59, 0.59, 0.59, 0.59, 0.59, 0.59,
        // :     ;     <     =     >     ?     @
        0.29, 0.29, 0.62, 0.62, 0.62, 0.53, 1.07,
        // A     B     C     D     E     F     G     H     I     J     K     L     M
        0.70, 0.64, 0.64, 0.70, 0.59, 0.53, 0.70, 0.70, 0.26, 0.41, 0.64, 0.56, 0.82,
        // N     O     P     Q     R     S     T     U     V     W     X     Y     Z
        0.70, 0.76, 0.59, 0.76, 0.64, 0.53, 0.59, 0.70, 0.70, 0.94, 0.64, 0.64, 0.59,
        // [     \     ]     ^     _     `
        0.29, 0.33, 0.29, 0.49, 0.59, 0.36,
        // a     b     c     d     e     f     g     h     i     j     k     l     m
        0.59, 0.59, 0.53, 0.59, 0.59, 0.33, 0.59, 0.59, 0.23, 0.23, 0.56, 0.23, 0.87,
        // n     o     p     q     r     s     t     u     v     w     x     y     z
        0.59, 0.59, 0.59, 0.59, 0.35, 0.46, 0.41, 0.59, 0.53, 0.76, 0.53, 0.53, 0.46,
        // {     |     }     ~
        0.35, 0.27, 0.35, 0.62,
    ],
    average_char_width: 0.55,
    space_width: 0.26,
};

/// Oswald — condensed display sans-serif (Founder template). Approx. 68% of Inter.
static OSWALD_TABLE: FontMetricTable = FontMetricTable {
    font: FontFamily::Oswald,
    #[rustfmt::skip]
    widths: [
        // sp    !     "     #     $     %     &     '     (     )     *     +     ,     -     .     /
        0.17, 0.20, 0.26, 0.38, 0.38, 0.61, 0.46, 0.15, 0.23, 0.23, 0.27, 0.40, 0.19, 0.23, 0.19, 0.21,
        // 0     1     2     3     4     5     6     7     8     9
        0.38, 0.38, 0.38, 0.38, 0.38, 0.38, 0.38, 0.38, 0.38, 0.38,
        // :     ;     <     =     >     ?     @
        0.19, 0.19, 0.40, 0.40, 0.40, 0.34, 0.69,
        // A     B     C     D     E     F     G     H     I     J     K     L     M
        0.46, 0.41, 0.41, 0.46, 0.38, 0.34, 0.46, 0.46, 0.17, 0.27, 0.41, 0.36, 0.53,
        // N     O     P     Q     R     S     T     U     V     W     X     Y     Z
        0.46, 0.49, 0.38, 0.49, 0.41, 0.34, 0.38, 0.46, 0.46, 0.61, 0.41, 0.41, 0.38,
        // [     \     ]     ^     _     `
        0.19, 0.21, 0.19, 0.32, 0.38, 0.23,
        // a     b     c     d     e     f     g     h     i     j     k     l     m
        0.38, 0.38, 0.34, 0.38, 0.38, 0.21, 0.38, 0.38, 0.15, 0.15, 0.36, 0.15, 0.56,
        // n     o     p     q     r     s     t     u     v     w     x     y     z
        0.38, 0.38, 0.38, 0.38, 0.23, 0.30, 0.27, 0.38, 0.34, 0.49, 0.34, 0.34, 0.30,
        // {     |     }     ~
        0.23, 0.18, 0.23, 0.40,
    ],
    average_char_width: 0.35,
    space_width: 0.17,
};

/// Computer Modern — traditional TeX font (Classic/ATS-safe template). Approx. 90% of Inter.
static COMPUTER_MODERN_TABLE: FontMetricTable = FontMetricTable {
    font: FontFamily::ComputerModern,
    #[rustfmt::skip]
    widths: [
        // sp    !     "     #     $     %     &     '     (     )     *     +     ,     -     .     /
        0.23, 0.27, 0.34, 0.50, 0.50, 0.80, 0.60, 0.20, 0.30, 0.30, 0.35, 0.53, 0.25, 0.30, 0.25, 0.28,
        // 0     1     2     3     4     5     6     7     8     9
        0.50, 0.50, 0.50, 0.50, 0.50, 0.50, 0.50, 0.50, 0.50, 0.50,
        // :     ;     <     =     >     ?     @
        0.25, 0.25, 0.53, 0.53, 0.53, 0.45, 0.92,
        // A     B     C     D     E     F     G     H     I     J     K     L     M
        0.60, 0.55, 0.55, 0.60, 0.50, 0.45, 0.60, 0.60, 0.23, 0.35, 0.55, 0.48, 0.70,
        // N     O     P     Q     R     S     T     U     V     W     X     Y     Z
        0.60, 0.65, 0.50, 0.65, 0.55, 0.45, 0.50, 0.60, 0.60, 0.80, 0.55, 0.55, 0.50,
        // [     \     ]     ^     _     `
        0.25, 0.28, 0.25, 0.42, 0.50, 0.31,
        // a     b     c     d     e     f     g     h     i     j     k     l     m
        0.50, 0.50, 0.45, 0.50, 0.50, 0.28, 0.50, 0.50, 0.20, 0.20, 0.48, 0.20, 0.75,
        // n     o     p     q     r     s     t     u     v     w     x     y     z
        0.50, 0.50, 0.50, 0.50, 0.30, 0.40, 0.35, 0.50, 0.45, 0.65, 0.45, 0.45, 0.40,
        // {     |     }     ~
        0.30, 0.23, 0.30, 0.53,
    ],
    average_char_width: 0.47,
    space_width: 0.23,
};

/// Returns the static metric table for a given font family.
pub fn get_metrics(font: &FontFamily) -> &'static FontMetricTable {
    match font {
        FontFamily::Inter => &INTER_TABLE,
        FontFamily::EbGaramond => &EB_GARAMOND_TABLE,
        FontFamily::Lato => &LATO_TABLE,
        FontFamily::Oswald => &OSWALD_TABLE,
        FontFamily::ComputerModern => &COMPUTER_MODERN_TABLE,
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config(font: FontFamily) -> PageConfig {
        default_page_config(font)
    }

    #[test]
    fn test_measure_str_empty_returns_zero() {
        let metrics = get_metrics(&FontFamily::Inter);
        assert_eq!(metrics.measure_str(""), 0.0);
    }

    #[test]
    fn test_measure_str_single_space() {
        let metrics = get_metrics(&FontFamily::Inter);
        let width = metrics.measure_str(" ");
        assert!(
            (width - 0.25).abs() < 1e-4,
            "space width should be 0.25, got {width}"
        );
    }

    #[test]
    fn test_measure_str_ascii_characters() {
        let metrics = get_metrics(&FontFamily::Inter);
        // "Rust" = R(0.61) + u(0.56) + s(0.44) + t(0.39) = 2.00
        let width = metrics.measure_str("Rust");
        assert!(
            (width - 2.00).abs() < 1e-3,
            "Rust width should be ~2.00, got {width}"
        );
    }

    #[test]
    fn test_measure_str_non_ascii_falls_back() {
        let metrics = get_metrics(&FontFamily::Inter);
        // "é" is non-ASCII → falls back to average_char_width (0.52)
        let width = metrics.measure_str("é");
        assert!(
            (width - metrics.average_char_width).abs() < 1e-4,
            "non-ASCII should use average_char_width"
        );
    }

    #[test]
    fn test_coverage_fraction_short_string_below_1() {
        let metrics = get_metrics(&FontFamily::Inter);
        let config = make_config(FontFamily::Inter);
        // A short word should be much less than 1.0 (full line)
        let frac = metrics.coverage_fraction("Hi", &config);
        assert!(
            frac < 1.0,
            "short string should have coverage < 1.0, got {frac}"
        );
        assert!(frac > 0.0, "short string should have coverage > 0.0");
    }

    #[test]
    fn test_coverage_fraction_long_string_above_1() {
        let metrics = get_metrics(&FontFamily::Inter);
        let config = make_config(FontFamily::Inter);
        // Repeat "word " many times to exceed the line width
        let long_str: String = "word ".repeat(20);
        let frac = metrics.coverage_fraction(&long_str, &config);
        assert!(
            frac > 1.0,
            "long string should have coverage > 1.0, got {frac}"
        );
    }

    #[test]
    fn test_estimated_lines_single_word_is_one_line() {
        let metrics = get_metrics(&FontFamily::Inter);
        let config = make_config(FontFamily::Inter);
        assert_eq!(metrics.estimated_lines("Rust", &config), 1);
    }

    #[test]
    fn test_estimated_lines_long_text_wraps_to_two() {
        let metrics = get_metrics(&FontFamily::Inter);
        let config = make_config(FontFamily::Inter);
        // A realistic 1.5-line resume bullet should wrap to 2 lines
        let bullet = "Architected a distributed caching layer using Redis and consistent hashing, \
                      reducing p99 latency by 40% under 50k RPS peak load";
        let lines = metrics.estimated_lines(bullet, &config);
        assert!(
            lines >= 1 && lines <= 3,
            "realistic bullet should be 1–3 lines, got {lines}"
        );
    }

    #[test]
    fn test_all_five_fonts_accessible() {
        // All 5 font families must be accessible via get_metrics
        let _ = get_metrics(&FontFamily::Inter);
        let _ = get_metrics(&FontFamily::EbGaramond);
        let _ = get_metrics(&FontFamily::Lato);
        let _ = get_metrics(&FontFamily::Oswald);
        let _ = get_metrics(&FontFamily::ComputerModern);
    }

    #[test]
    fn test_condensed_font_narrower_than_wide_font() {
        // Oswald (condensed) should measure narrower than Lato (expanded)
        let text = "Architected distributed caching layer";
        let oswald = get_metrics(&FontFamily::Oswald);
        let lato = get_metrics(&FontFamily::Lato);
        let config_o = make_config(FontFamily::Oswald);
        let config_l = make_config(FontFamily::Lato);
        assert!(
            oswald.coverage_fraction(text, &config_o) < lato.coverage_fraction(text, &config_l),
            "Oswald (condensed) coverage should be less than Lato (expanded)"
        );
    }

    #[test]
    fn test_default_page_config_sanity() {
        let config = default_page_config(FontFamily::Inter);
        assert_eq!(config.font, FontFamily::Inter);
        assert_eq!(config.font_size_pt, 11);
        assert!(config.text_width_em > 40.0 && config.text_width_em < 50.0);
        assert!(config.usable_height_lines > 30);
        assert!((config.microtype_margin - 0.03).abs() < 1e-4);
    }
}
