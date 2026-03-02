#![allow(dead_code)]
//! Tectonic LaTeX → PDF compilation via `tokio::process::Command`.
//!
//! Invocation: `tectonic - --outfmt pdf`
//! Stdin: raw LaTeX source bytes.
//! Stdout: raw PDF bytes.
//! TempDir: Tectonic writes aux files (.fmt, .aux, .xdv) here; auto-cleaned on drop.
//!
//! NOTE: tokio::process::Command is I/O-bound, not CPU-bound.
//! No spawn_blocking needed here — async I/O suffices.

use std::time::Instant;

use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use uuid::Uuid;

use crate::render::types::{RenderError, TectonicResult};

/// Compiles a LaTeX source string to PDF bytes using Tectonic.
///
/// Uses stdin mode (`tectonic -`) to avoid writing the .tex source to disk.
/// A TempDir is still created for Tectonic's internal aux files.
/// Times out after 30 seconds.
pub async fn compile_latex(
    latex_source: &str,
    _job_id: Uuid,
) -> Result<TectonicResult, RenderError> {
    let tmp_dir = TempDir::new()?;
    let start = Instant::now();

    let mut child = Command::new("tectonic")
        .arg("-")
        .arg("--outfmt")
        .arg("pdf")
        .current_dir(tmp_dir.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RenderError::TectonicNotFound
            } else {
                RenderError::Io(e)
            }
        })?;

    // Write source to stdin, then close it to signal EOF to tectonic
    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(latex_source.as_bytes()).await?;
        // Drop closes the pipe; tectonic reads EOF and begins compilation
    }

    let output = timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .map_err(|_| RenderError::CompilationFailed {
            exit_code: -1,
            stderr: "Tectonic compilation timed out after 30 seconds".to_string(),
        })??;

    let duration_ms = start.elapsed().as_millis() as u64;
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(RenderError::CompilationFailed {
            exit_code: output.status.code().unwrap_or(-1),
            stderr,
        });
    }

    if output.stdout.is_empty() {
        return Err(RenderError::EmptyPdf);
    }

    // TempDir dropped here → aux files cleaned up automatically
    Ok(TectonicResult {
        pdf_bytes: output.stdout,
        stderr,
        duration_ms,
    })
}

/// Checks that the `tectonic` binary is available on PATH.
///
/// Called at server startup — fails fast so the operator knows to install tectonic.
pub async fn check_tectonic_available() -> Result<(), RenderError> {
    let result = Command::new("tectonic")
        .arg("--version")
        .output()
        .await
        .map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                RenderError::TectonicNotFound
            } else {
                RenderError::Io(e)
            }
        })?;

    if !result.status.success() {
        return Err(RenderError::TectonicNotFound);
    }

    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Shadows PATH with an empty string so `tectonic` binary cannot be found.
    /// Verifies that `check_tectonic_available` returns TectonicNotFound.
    #[tokio::test]
    async fn test_check_tectonic_not_found_on_empty_path() {
        // Override PATH so the tectonic binary cannot be resolved
        std::env::set_var("PATH", "");
        let result = check_tectonic_available().await;
        // Restore PATH so later tests are not affected
        std::env::remove_var("PATH");

        assert!(
            matches!(
                result,
                Err(RenderError::TectonicNotFound) | Err(RenderError::Io(_))
            ),
            "expected TectonicNotFound or Io on empty PATH, got: {:?}",
            result
        );
    }

    /// Requires the `tectonic` binary on PATH. Skipped in CI unless tectonic is installed.
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

    /// Requires the `tectonic` binary on PATH. Skipped in CI unless tectonic is installed.
    #[tokio::test]
    #[ignore]
    async fn test_compile_valid_latex_returns_pdf() {
        let valid_latex = r#"\documentclass{article}
\begin{document}
Hello, Templar.
\end{document}"#;
        let result = compile_latex(valid_latex, Uuid::new_v4()).await;
        let tectonic_result = result.expect("valid LaTeX must compile successfully");
        assert!(
            !tectonic_result.pdf_bytes.is_empty(),
            "PDF bytes must be non-empty"
        );
        // PDF magic bytes: %PDF-
        assert!(
            tectonic_result.pdf_bytes.starts_with(b"%PDF-"),
            "output must be a valid PDF"
        );
    }
}
