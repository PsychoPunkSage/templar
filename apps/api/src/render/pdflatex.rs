#![allow(dead_code)]
//! pdflatex LaTeX → PDF compilation via `tokio::process::Command`.
//!
//! Invocation: `pdflatex -interaction=nonstopmode -halt-on-error -output-directory <dir> <tex_path>`
//! Input: .tex file written to a TempDir.
//! Output: .pdf file written to the same TempDir by pdflatex.
//!
//! NOTE: tokio::process::Command is I/O-bound, not CPU-bound.
//! No spawn_blocking needed here — async I/O suffices.

use std::time::Instant;

use tempfile::TempDir;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::render::types::{PdflatexResult, RenderError};

/// Compiles a LaTeX source string to PDF bytes using pdflatex.
///
/// Writes `latex_source` to a temporary `document.tex` file, invokes
/// `pdflatex -interaction=nonstopmode -halt-on-error -output-directory <dir> document.tex`,
/// then reads back `document.pdf` from the same directory.
/// The TempDir is dropped at the end of the function, cleaning up all files.
///
/// Key design decisions:
/// - `-interaction=nonstopmode`: pdflatex never pauses for user input on errors;
///   it logs the error and continues (or exits with a non-zero code on `-halt-on-error`).
///
/// - `-halt-on-error`: exit non-zero on the first LaTeX error rather than attempting
///   to continue compiling a broken document. Ensures fast failure with a clear exit code.
///
/// - `-output-directory`: pdflatex writes the .pdf, .log, and .aux files to the
///   specified directory rather than the current directory. Using the TempDir ensures
///   all output files are cleaned up automatically when the TempDir is dropped.
///
/// - No network access: TeX Live packages are installed on disk via apt at Docker build
///   time. pdflatex never downloads anything at runtime — no cache hash to manage,
///   no format invalidation, no network dependency.
///
/// - Child process is killed on timeout: `tokio::timeout()` cancels the Rust
///   future but leaves the OS child process running. Without an explicit kill,
///   every timed-out pdflatex process continues running in the background.
///   We always kill before returning a timeout error.
pub async fn compile_latex(
    latex_source: &str,
    job_id: Uuid,
) -> Result<PdflatexResult, RenderError> {
    let tmp_dir = TempDir::new().map_err(RenderError::Io)?;
    let start = Instant::now();

    let tex_path = tmp_dir.path().join("document.tex");
    let pdf_path = tmp_dir.path().join("document.pdf");
    tokio::fs::write(&tex_path, latex_source.as_bytes())
        .await
        .map_err(RenderError::Io)?;

    tracing::info!(
        job_id = %job_id,
        tex_path = %tex_path.display(),
        "pdflatex: spawning process"
    );

    let mut child = Command::new("pdflatex")
        // Never pause for user input — exit or log on errors
        .arg("-interaction=nonstopmode")
        // Exit non-zero on the first LaTeX error
        .arg("-halt-on-error")
        // Write .pdf, .log, .aux to the TempDir (auto-cleaned on drop)
        .arg("-output-directory")
        .arg(tmp_dir.path())
        .arg(&tex_path)
        .current_dir(tmp_dir.path())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RenderError::PdflatexNotFound
            } else {
                RenderError::Io(e)
            }
        })?;

    // Take both stdout and stderr handles BEFORE calling child.wait().
    // We use child.wait() (takes &mut self) instead of child.wait_with_output() (takes self)
    // because wait_with_output() consumes the child, making it impossible to call
    // child.kill() if the timeout fires. By using wait() + separate handle reading,
    // we retain ownership of `child` all the way through the timeout check.
    let mut stdout_handle = child.stdout.take();
    let mut stderr_handle = child.stderr.take();

    // Wait for pdflatex to finish, but cap at 120 seconds.
    // If it times out we MUST kill the child process. tokio::timeout() only cancels
    // the Rust future — the OS child process keeps running. Without an explicit kill(),
    // every timed-out pdflatex continues consuming CPU and disk I/O, and the next
    // attempt will be equally slow until the container is restarted.
    let exit_status = match timeout(Duration::from_secs(120), child.wait()).await {
        Ok(Ok(status)) => status,
        Ok(Err(e)) => return Err(RenderError::Io(e)),
        Err(_) => {
            tracing::error!(
                job_id = %job_id,
                timeout_secs = 120,
                "pdflatex: timeout — killing child process"
            );
            let _ = child.kill().await;
            return Err(RenderError::CompilationFailed {
                exit_code: -1,
                stderr: "pdflatex compilation timed out after 120 seconds".to_string(),
            });
        }
    };

    // Process exited — collect stdout + stderr (fully flushed now that the process is done).
    // Read both before the TempDir drops so they are captured even on failure.
    let mut stdout_bytes = Vec::new();
    if let Some(ref mut handle) = stdout_handle {
        let _ = handle.read_to_end(&mut stdout_bytes).await;
    }
    let mut stderr_bytes = Vec::new();
    if let Some(ref mut handle) = stderr_handle {
        let _ = handle.read_to_end(&mut stderr_bytes).await;
    }

    // Read the .log file BEFORE TempDir drops — it contains the full LaTeX transcript,
    // including package errors, missing font messages, and undefined control sequences
    // that are the root cause of most pdflatex failures.
    let log_path = tmp_dir.path().join("document.log");
    let log_text = tokio::fs::read_to_string(&log_path)
        .await
        .unwrap_or_default();

    let duration_ms = start.elapsed().as_millis() as u64;
    let stdout_text = String::from_utf8_lossy(&stdout_bytes).to_string();
    let stderr_text = String::from_utf8_lossy(&stderr_bytes).to_string();

    // Combine all diagnostic output into a single error string for maximum debuggability.
    // Order: stderr first (pdflatex writes errors there), then stdout (progress notes),
    // then the .log file (full transcript). Truncate log to 8 KB so the DB column
    // and error message stay manageable.
    let build_combined_error = |exit_code: i32| -> String {
        let mut combined = String::new();
        if !stderr_text.is_empty() {
            combined.push_str("=== stderr ===\n");
            combined.push_str(&stderr_text);
            combined.push('\n');
        }
        if !stdout_text.is_empty() {
            combined.push_str("=== stdout ===\n");
            combined.push_str(&stdout_text);
            combined.push('\n');
        }
        if !log_text.is_empty() {
            combined.push_str("=== document.log (first 8 KB) ===\n");
            let truncated: String = log_text.chars().take(8192).collect();
            combined.push_str(&truncated);
            combined.push('\n');
        }
        if combined.is_empty() {
            combined = format!("pdflatex exited with code {exit_code} but produced no output");
        }
        combined
    };

    if !exit_status.success() {
        let exit_code = exit_status.code().unwrap_or(-1);
        let combined_error = build_combined_error(exit_code);
        // Log full diagnostics here so they appear in container logs even if the caller
        // does not log them. The full text is critical for diagnosing missing fonts,
        // package errors, and LaTeX syntax problems.
        tracing::error!(
            job_id = %job_id,
            exit_code = exit_code,
            duration_ms = duration_ms,
            diagnostics = %combined_error,
            "pdflatex process exited with non-zero status"
        );
        return Err(RenderError::CompilationFailed {
            exit_code,
            stderr: combined_error,
        });
    }

    tracing::info!(
        job_id = %job_id,
        duration_ms = duration_ms,
        "pdflatex process exited successfully"
    );

    // pdflatex writes document.pdf to the output-directory (tmp_dir).
    // Read the PDF before dropping TempDir (which cleans up the directory).
    let pdf_bytes =
        tokio::fs::read(&pdf_path)
            .await
            .map_err(|e| RenderError::CompilationFailed {
                exit_code: -1,
                stderr: format!("Failed to read output PDF: {e}"),
            })?;

    if pdf_bytes.is_empty() {
        return Err(RenderError::EmptyPdf);
    }

    // Build a combined warnings string from stderr + log for the success path.
    // pdflatex writes overfull/underfull hbox warnings to stdout on success.
    let warnings = {
        let mut w = String::new();
        if !stderr_text.is_empty() {
            w.push_str(&stderr_text);
        }
        if !stdout_text.is_empty() {
            if !w.is_empty() {
                w.push('\n');
            }
            w.push_str(&stdout_text);
        }
        w
    };

    // TempDir dropped here → all .tex, .pdf, .log, .aux files cleaned up automatically
    Ok(PdflatexResult {
        pdf_bytes,
        stderr: warnings,
        duration_ms,
    })
}

/// Checks that the `pdflatex` binary is available on PATH.
///
/// Called at server startup — fails fast so the operator knows to install TeX Live.
pub async fn check_pdflatex_available() -> Result<(), RenderError> {
    let result = Command::new("pdflatex")
        .arg("--version")
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RenderError::PdflatexNotFound
            } else {
                RenderError::Io(e)
            }
        })?;

    if !result.status.success() {
        return Err(RenderError::PdflatexNotFound);
    }

    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Shadows PATH with an empty string so `pdflatex` binary cannot be found.
    /// Verifies that `check_pdflatex_available` returns PdflatexNotFound.
    #[tokio::test]
    async fn test_check_pdflatex_not_found_on_empty_path() {
        // Override PATH so the pdflatex binary cannot be resolved
        std::env::set_var("PATH", "");
        let result = check_pdflatex_available().await;
        // Restore PATH so later tests are not affected
        std::env::remove_var("PATH");

        assert!(
            matches!(
                result,
                Err(RenderError::PdflatexNotFound) | Err(RenderError::Io(_))
            ),
            "expected PdflatexNotFound or Io on empty PATH, got: {:?}",
            result
        );
    }

    /// Requires `pdflatex` on PATH. Skipped in CI unless TeX Live is installed.
    #[tokio::test]
    #[ignore]
    async fn test_compile_invalid_latex_returns_error() {
        let bad_latex = r"\documentclass{article}\begin{document}\BADINVALIDCOMMAND\end{document}";
        let result = compile_latex(bad_latex, Uuid::new_v4()).await;
        assert!(
            matches!(result, Err(RenderError::CompilationFailed { .. })),
            "invalid LaTeX must return CompilationFailed, got: {:?}",
            result
        );
    }

    /// Requires `pdflatex` on PATH. Skipped in CI unless TeX Live is installed.
    #[tokio::test]
    #[ignore]
    async fn test_compile_valid_latex_returns_pdf() {
        let valid_latex = r#"\documentclass{article}
\begin{document}
Hello, Templar.
\end{document}"#;
        let result = compile_latex(valid_latex, Uuid::new_v4()).await;
        let pdf_result = result.expect("valid LaTeX must compile successfully");
        assert!(
            !pdf_result.pdf_bytes.is_empty(),
            "PDF bytes must be non-empty"
        );
        // PDF magic bytes: %PDF-
        assert!(
            pdf_result.pdf_bytes.starts_with(b"%PDF-"),
            "output must be a valid PDF"
        );
    }
}
