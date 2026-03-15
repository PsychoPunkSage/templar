//! File text extractor: converts uploaded file bytes into plain UTF-8 text.
//!
//! Supported formats:
//! - `.md` / `.txt` — direct UTF-8 decode
//! - `.pdf`         — text extraction via `pdf-extract` crate
//!
//! A hard 10 MB size limit is enforced before any processing.

use std::path::Path;

use crate::errors::AppError;

const MAX_FILE_SIZE: usize = 10 * 1024 * 1024; // 10 MB

/// Extract plain text from an uploaded file.
///
/// Dispatches based on file extension (case-insensitive):
/// - `.md` / `.txt` — `String::from_utf8`
/// - `.pdf`         — `pdf_extract::extract_text_from_mem`
/// - anything else  — `AppError::Validation` with an informative message
///
/// Returns `AppError::Validation` if the file exceeds MAX_FILE_SIZE or the
/// file type is unsupported.
pub fn extract_text(filename: &str, bytes: &[u8]) -> Result<String, AppError> {
    if bytes.len() > MAX_FILE_SIZE {
        return Err(AppError::Validation(format!(
            "File too large: {:.1} MB. Maximum allowed size is 10 MB.",
            bytes.len() as f64 / (1024.0 * 1024.0)
        )));
    }

    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "md" | "txt" => String::from_utf8(bytes.to_vec()).map_err(|_| {
            AppError::Validation(
                "File must be valid UTF-8 text. Ensure the file is a plain text or Markdown file."
                    .into(),
            )
        }),
        "pdf" => pdf_extract::extract_text_from_mem(bytes).map_err(|e| {
            AppError::Validation(format!(
                "PDF text extraction failed: {e}. \
                 Ensure the PDF contains selectable text (not a scanned image)."
            ))
        }),
        other => Err(AppError::Validation(format!(
            "Unsupported file type '.{other}'. Accepted formats: .md, .txt, .pdf"
        ))),
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_md_file() {
        let content = "# My Resume\n\nWorked at ACME Corp 2020–2023.";
        let result = extract_text("resume.md", content.as_bytes());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), content);
    }

    #[test]
    fn test_extract_txt_file() {
        let content = "Plain text resume content.\nMultiple lines here.";
        let result = extract_text("export.txt", content.as_bytes());
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), content);
    }

    #[test]
    fn test_case_insensitive_extension() {
        let content = "Resume content here.";
        // .MD uppercase
        let result = extract_text("RESUME.MD", content.as_bytes());
        assert!(result.is_ok());
        // .TXT uppercase
        let result = extract_text("notes.TXT", content.as_bytes());
        assert!(result.is_ok());
    }

    #[test]
    fn test_unsupported_extension_returns_error() {
        let result = extract_text("document.docx", b"some bytes");
        assert!(matches!(result, Err(AppError::Validation(_))));
        if let Err(AppError::Validation(msg)) = result {
            assert!(msg.contains("docx"));
            assert!(msg.contains(".md"));
        }
    }

    #[test]
    fn test_no_extension_returns_error() {
        let result = extract_text("noextension", b"some bytes");
        assert!(matches!(result, Err(AppError::Validation(_))));
    }

    #[test]
    fn test_file_too_large_returns_error() {
        // 11 MB of zeros
        let big_bytes = vec![0u8; 11 * 1024 * 1024];
        let result = extract_text("large.txt", &big_bytes);
        assert!(matches!(result, Err(AppError::Validation(_))));
        if let Err(AppError::Validation(msg)) = result {
            assert!(msg.contains("10 MB"));
        }
    }

    #[test]
    fn test_invalid_utf8_txt_returns_error() {
        // Invalid UTF-8 bytes
        let invalid = vec![0xFF, 0xFE, 0x00, 0x01];
        let result = extract_text("bad.txt", &invalid);
        assert!(matches!(result, Err(AppError::Validation(_))));
        if let Err(AppError::Validation(msg)) = result {
            assert!(msg.contains("UTF-8"));
        }
    }

    #[test]
    fn test_exactly_max_size_is_allowed() {
        // Exactly 10 MB of valid ASCII
        let exactly_max = vec![b'a'; MAX_FILE_SIZE];
        let result = extract_text("exact.txt", &exactly_max);
        assert!(result.is_ok());
    }
}
