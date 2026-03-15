//! File-based template system for Templar.
//!
//! Templates live in a directory on disk (one subdirectory per template).
//! Each subdirectory contains:
//!   - `template.tex`    — the LaTeX source with `{{PLACEHOLDER}}` tokens
//!   - `metadata.json`   — id, name, description, tags, engine, thumbnail_s3_key
//!   - `sample_data.json`— sample profile + bullets used for thumbnail generation
//!
//! At startup, `load_templates_from_dir()` reads all valid subdirectories into
//! an in-memory `TemplateCache`. The cache is read-only after startup — adding a
//! new template requires a restart, which is acceptable for the MVP.
//!
//! This module is intentionally self-contained: it does not import from generation/,
//! render/, or grounding/. The only shared dependency is `escape_latex` from render/templates.

pub mod handlers;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::render::templates::escape_latex;

// ────────────────────────────────────────────────────────────────────────────
// Public types
// ────────────────────────────────────────────────────────────────────────────

/// Metadata about a single template, loaded from `metadata.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TemplateMetadata {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    /// LaTeX engine required: "pdflatex" or "xelatex".
    /// Determines which Tectonic compile path to use.
    pub engine: String,
    /// S3 key for the pre-generated thumbnail PNG.
    pub thumbnail_s3_key: String,
}

/// A user's contact/profile information, used to fill header placeholders.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProfileData {
    pub full_name: String,
    pub email: String,
    pub phone: String,
    pub location: String,
    /// LinkedIn handle or URL path (e.g. "in/username")
    pub linkedin: String,
    /// Personal website URL (optional)
    pub website: String,
}

/// A single section of sample content used to render template thumbnails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleSection {
    pub name: String,
    pub bullets: Vec<String>,
}

/// Parsed `sample_data.json` — used exclusively for thumbnail generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleData {
    pub profile: ProfileData,
    pub sections: Vec<SampleSection>,
}

/// A fully loaded template: metadata + raw LaTeX source + sample data.
///
/// `Arc` here because the cache holds an `Arc<LoadedTemplate>` per entry,
/// and handlers clone the Arc (not the struct) when they need a reference.
#[derive(Debug, Clone)]
pub struct LoadedTemplate {
    pub metadata: TemplateMetadata,
    /// Raw LaTeX source as read from `template.tex`. Contains `{{PLACEHOLDER}}` tokens.
    pub latex_source: String,
    /// Sample data used for thumbnail generation.
    pub sample_data: SampleData,
}

/// The in-memory template registry.
///
/// We use `tokio::sync::RwLock` (async-aware) instead of `std::sync::RwLock`.
/// std's RwLock blocks the OS thread while held — in an async context this would
/// starve the Tokio executor. The template cache is read-heavy (every GET /templates
/// request) and written only once at startup, so RwLock >> Mutex for this workload.
pub type TemplateCache = tokio::sync::RwLock<HashMap<String, Arc<LoadedTemplate>>>;

/// Summary returned to API consumers — no LaTeX source, just UI-relevant fields.
#[derive(Debug, Clone, Serialize)]
pub struct TemplateSummary {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    /// API path for the thumbnail PNG: `/api/v1/templates/{id}/preview`
    pub thumbnail_url: String,
}

// ────────────────────────────────────────────────────────────────────────────
// Directory loader
// ────────────────────────────────────────────────────────────────────────────

/// Loads all templates from a root directory at server startup.
///
/// Each subdirectory of `dir` that contains `template.tex`, `metadata.json`,
/// and `sample_data.json` is loaded as one template. Invalid/incomplete
/// directories are skipped with a warning — they do NOT crash the server.
///
/// Returns an empty map if the directory does not exist (graceful degradation
/// for development environments without templates deployed).
pub fn load_templates_from_dir(dir: &Path) -> Result<HashMap<String, Arc<LoadedTemplate>>> {
    let start = std::time::Instant::now();
    let mut templates = HashMap::new();

    if !dir.exists() {
        // In local dev, templates directory may not be present yet. Warn and continue.
        warn!(
            "Templates directory '{}' does not exist — no file-based templates loaded",
            dir.display()
        );
        return Ok(templates);
    }

    info!(dir = %dir.display(), "Loading templates from directory");

    let entries = std::fs::read_dir(dir)
        .with_context(|| format!("Failed to read templates directory: {}", dir.display()))?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                warn!("Skipping unreadable templates entry: {e}");
                continue;
            }
        };

        let path = entry.path();
        if !path.is_dir() {
            // Skip loose files in the root templates directory
            continue;
        }

        match load_single_template(&path) {
            Ok(t) => {
                let id = t.metadata.id.clone();
                info!("Loaded template '{}' from '{}'", id, path.display());
                templates.insert(id, Arc::new(t));
            }
            Err(e) => {
                // A bad template directory is a warning, not a startup failure.
                // Production deploys should review logs, but a single corrupt
                // template should never take down the entire server.
                warn!(
                    "Skipping invalid template directory '{}': {e}",
                    path.display()
                );
            }
        }
    }

    info!(
        count = templates.len(),
        duration_ms = start.elapsed().as_millis() as u64,
        dir = %dir.display(),
        "Template loading complete"
    );

    Ok(templates)
}

/// Loads one template from its directory. Returns an error if any required file
/// is missing or malformed.
fn load_single_template(dir: &Path) -> Result<LoadedTemplate> {
    let tex_path = dir.join("template.tex");
    let meta_path = dir.join("metadata.json");
    let sample_path = dir.join("sample_data.json");

    let latex_source = std::fs::read_to_string(&tex_path)
        .with_context(|| format!("Missing template.tex in {}", dir.display()))?;

    let metadata_str = std::fs::read_to_string(&meta_path)
        .with_context(|| format!("Missing metadata.json in {}", dir.display()))?;
    let metadata: TemplateMetadata = serde_json::from_str(&metadata_str)
        .with_context(|| format!("Invalid metadata.json in {}", dir.display()))?;

    let sample_str = std::fs::read_to_string(&sample_path)
        .with_context(|| format!("Missing sample_data.json in {}", dir.display()))?;
    let sample_data: SampleData = serde_json::from_str(&sample_str)
        .with_context(|| format!("Invalid sample_data.json in {}", dir.display()))?;

    Ok(LoadedTemplate {
        metadata,
        latex_source,
        sample_data,
    })
}

// ────────────────────────────────────────────────────────────────────────────
// Template rendering
// ────────────────────────────────────────────────────────────────────────────

/// Builds a complete LaTeX document by substituting profile + section content
/// into the template's `{{PLACEHOLDER}}` tokens.
///
/// Using str::replace() for placeholder substitution is safe here because:
///   1. Placeholders (`{{FULL_NAME}}` etc.) are defined by us, not user-controlled
///   2. `escape_latex()` is called on VALUES before substitution — not on the template
///   3. Placeholders don't overlap (no placeholder is a prefix of another)
///   4. The template author (us) controls the .tex files — not arbitrary users
///      A regex approach would be slower for no benefit given these constraints.
pub fn render_file_template(
    template: &LoadedTemplate,
    profile: &ProfileData,
    sections: &[SampleSection],
) -> String {
    // Build the contact line — only include fields that are non-empty.
    // This avoids orphaned separators when optional fields (linkedin, website) are blank.
    let contact_line = build_contact_line(profile);

    // Build the sections LaTeX block (same format as build_latex_document's body)
    let sections_latex = build_sections_latex(sections);

    // Apply all placeholder substitutions in a single chained replace.
    // Each value is already LaTeX-escaped before substitution.
    template
        .latex_source
        // Full name: large bold header — must be escaped (names can contain & % etc.)
        .replace("{{FULL_NAME}}", &escape_latex(&profile.full_name))
        // Contact line is pre-built with icons + escaped values; insert verbatim
        .replace("{{CONTACT_LINE}}", &contact_line)
        // Sections block is pre-built and pre-escaped; insert verbatim
        .replace("{{SECTIONS}}", &sections_latex)
}

/// Renders the template with sample data for thumbnail generation.
///
/// Identical to `render_file_template` but uses the sample profile + sections
/// bundled with the template. Called at startup for thumbnail pre-computation.
pub fn render_template_with_sample(template: &LoadedTemplate) -> String {
    // Convert SampleSection → the same type (they ARE the same type here)
    render_file_template(
        template,
        &template.sample_data.profile,
        &template.sample_data.sections,
    )
}

/// Builds the `{{CONTACT_LINE}}` value: a comma-separated sequence of contact
/// fields, each prefixed with a fontawesome5 icon.
///
/// Only non-empty fields are included so the line never has orphaned separators
/// like "| |" between absent fields.
fn build_contact_line(profile: &ProfileData) -> String {
    let mut parts: Vec<String> = Vec::new();

    if !profile.email.is_empty() {
        // Email: use \href so it's clickable in the PDF
        parts.push(format!(
            r"\faEnvelope\ \href{{mailto:{email}}}{{{escaped}}}",
            email = profile.email,
            escaped = escape_latex(&profile.email),
        ));
    }
    if !profile.phone.is_empty() {
        parts.push(format!(r"\faPhone*\ {}", escape_latex(&profile.phone)));
    }
    if !profile.location.is_empty() {
        parts.push(format!(
            r"\faMapMarkerAlt\ {}",
            escape_latex(&profile.location)
        ));
    }
    if !profile.linkedin.is_empty() {
        // LinkedIn: wrap as hyperlink if it looks like a path (starts with "in/")
        let display = escape_latex(&profile.linkedin);
        parts.push(format!(r"\faLinkedin\ {display}"));
    }
    if !profile.website.is_empty() {
        let display = escape_latex(&profile.website);
        parts.push(format!(r"\faGlobe\ {display}"));
    }

    // Join with a spaced bar separator — LaTeX \quad gives even visual spacing
    parts.join(r" \quad\textbar\quad ")
}

/// Builds the `{{SECTIONS}}` value: `\section*{} + \begin{itemize}...\end{itemize}`
/// blocks for each section.
///
/// All bullet text is run through `escape_latex()` here.
fn build_sections_latex(sections: &[SampleSection]) -> String {
    let item_opts = "leftmargin=1.5em, itemsep=1pt, parsep=0pt, topsep=2pt";
    let mut out = String::new();

    for section in sections {
        if section.bullets.is_empty() {
            continue;
        }
        out.push_str(&format!("\\section*{{{}}}\n", escape_latex(&section.name)));
        out.push_str(&format!("\\begin{{itemize}}[{}]\n", item_opts));
        for bullet in &section.bullets {
            out.push_str(&format!("  \\item {}\n", escape_latex(bullet)));
        }
        out.push_str("\\end{itemize}\n\n");
    }

    out
}

// ────────────────────────────────────────────────────────────────────────────
// Thumbnail pre-computation (spawned as background task from main.rs)
// ────────────────────────────────────────────────────────────────────────────

/// Pre-computes and uploads PNG thumbnails for all loaded templates.
///
/// Design decisions:
/// - Runs as a fire-and-forget background task — does NOT block server readiness.
///   Server can answer requests while thumbnails are being computed.
/// - Idempotent: checks S3 before computing. Restarting the server is cheap.
/// - If pdftoppm is unavailable (local dev without poppler-utils), it logs a
///   warning and skips — the frontend falls back gracefully to a placeholder.
pub async fn precompute_thumbnails(
    cache: Arc<TemplateCache>,
    s3: aws_sdk_s3::Client,
    s3_bucket: String,
) {
    // Snapshot the template IDs we need to process (quick read lock, released immediately)
    let templates: Vec<(String, Arc<LoadedTemplate>)> = {
        let guard = cache.read().await;
        guard
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    };

    for (template_id, template) in templates {
        let thumbnail_key = template.metadata.thumbnail_s3_key.clone();

        // IMPORTANT: S3 HEAD check before running pdflatex. Without this guard,
        // every API restart would re-render and re-upload all thumbnails — expensive.
        // The HEAD check is cheap (<10ms) and makes thumbnail generation idempotent.
        if s3_object_exists(&s3, &s3_bucket, &thumbnail_key).await {
            info!(
                template_id = %template_id,
                key = %thumbnail_key,
                "Thumbnail already in S3 — skipping pre-computation"
            );
            continue;
        }

        info!(template_id = %template_id, "Starting thumbnail pre-computation");

        // Render sample LaTeX — this is CPU-cheap (just string ops)
        let latex = render_template_with_sample(&template);

        // Compile LaTeX → PDF in a blocking thread (Tectonic is CPU-bound)
        let job_id = uuid::Uuid::new_v4();
        let pdf_result = tokio::task::spawn_blocking(move || {
            // We run a synchronous compile here by using the async version via block_in_place
            // Actually, compile_latex is async, so we can't call it directly from spawn_blocking.
            // Instead, return the latex for the caller to compile.
            latex
        })
        .await;

        let latex = match pdf_result {
            Ok(l) => l,
            Err(e) => {
                warn!(template_id = %template_id, "Thumbnail task panicked: {e}");
                continue;
            }
        };

        // compile_latex is async — call it directly on the async runtime
        let compile_start = std::time::Instant::now();
        info!(template_id = %template_id, "Compiling template preview PDF via pdflatex");
        let compile_result = crate::render::pdflatex::compile_latex(&latex, job_id).await;
        let pdflatex_result = match compile_result {
            Ok(r) => {
                info!(
                    template_id = %template_id,
                    duration_ms = compile_start.elapsed().as_millis() as u64,
                    pdf_bytes = r.pdf_bytes.len(),
                    "Template preview PDF compiled successfully"
                );
                r
            }
            Err(e) => {
                warn!(
                    template_id = %template_id,
                    duration_ms = compile_start.elapsed().as_millis() as u64,
                    error = %e,
                    "Thumbnail compile failed — thumbnail will not be available"
                );
                continue;
            }
        };

        // Convert PDF → PNG using pdftoppm (requires poppler-utils in Docker image)
        let png_result = pdf_to_png(pdflatex_result.pdf_bytes).await;
        let png_bytes = match png_result {
            Ok(b) => b,
            Err(e) => {
                warn!(
                    "Thumbnail PNG conversion failed for '{}': {e} — pdftoppm may not be installed",
                    template_id
                );
                continue;
            }
        };

        // Upload PNG to S3
        match upload_thumbnail(&s3, &s3_bucket, &thumbnail_key, png_bytes).await {
            Ok(_) => info!(
                "Thumbnail for '{}' uploaded to S3: '{}'",
                template_id, thumbnail_key
            ),
            Err(e) => warn!("Thumbnail S3 upload failed for '{}': {e}", template_id),
        }
    }
}

/// Returns true if an object exists in S3 at the given key.
/// Uses HEAD (metadata only) — does not download the object body.
async fn s3_object_exists(s3: &aws_sdk_s3::Client, bucket: &str, key: &str) -> bool {
    s3.head_object()
        .bucket(bucket)
        .key(key)
        .send()
        .await
        .is_ok()
}

/// Converts PDF bytes to a PNG thumbnail using `pdftoppm` from poppler-utils.
///
/// `pdftoppm -png -r 96 -singlefile` renders only page 1 at 96 DPI.
/// 96 DPI gives a ~794×1123 pixel image — small enough for fast loading,
/// large enough to show the template layout clearly.
async fn pdf_to_png(pdf_bytes: Vec<u8>) -> Result<Vec<u8>> {
    use tokio::process::Command;

    // Write PDF to a temp file — pdftoppm requires a file path, not stdin
    let pdf_tmp = tempfile::NamedTempFile::new().context("Failed to create temp PDF file")?;
    let pdf_path = pdf_tmp.path().to_owned();
    std::fs::write(&pdf_path, &pdf_bytes).context("Failed to write PDF to temp file")?;

    // Output prefix: pdftoppm appends "-1.png" for page 1 + "-singlefile" suppresses page number
    let out_tmp = tempfile::tempdir().context("Failed to create temp output dir")?;
    let out_prefix = out_tmp.path().join("thumb");
    let out_prefix_str = out_prefix.to_str().context("Non-UTF-8 temp path")?;

    let status = Command::new("pdftoppm")
        .args([
            "-png",
            "-r",
            "96",
            "-singlefile",
            pdf_path.to_str().unwrap_or(""),
            out_prefix_str,
        ])
        .status()
        .await
        .context("pdftoppm not found — install poppler-utils")?;

    if !status.success() {
        anyhow::bail!("pdftoppm exited with status {}", status);
    }

    // With -singlefile, output file is <prefix>.png (no page number suffix)
    let png_path = PathBuf::from(format!("{out_prefix_str}.png"));
    let bytes = std::fs::read(&png_path)
        .with_context(|| format!("pdftoppm output not found at {}", png_path.display()))?;

    Ok(bytes)
}

/// Uploads a PNG to S3 at the given key.
async fn upload_thumbnail(
    s3: &aws_sdk_s3::Client,
    bucket: &str,
    key: &str,
    png_bytes: Vec<u8>,
) -> Result<()> {
    use aws_sdk_s3::primitives::ByteStream;

    s3.put_object()
        .bucket(bucket)
        .key(key)
        .body(ByteStream::from(png_bytes))
        .content_type("image/png")
        // Public-read caching hint — the thumbnail PNG is static content
        .cache_control("public, max-age=86400")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("S3 upload error: {e}"))?;

    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_profile() -> ProfileData {
        ProfileData {
            full_name: "Jane Doe".to_string(),
            email: "jane@example.com".to_string(),
            phone: "+1 555 000 0000".to_string(),
            location: "New York, NY".to_string(),
            linkedin: "in/janedoe".to_string(),
            website: "".to_string(), // empty — should be omitted from contact line
        }
    }

    fn sample_sections() -> Vec<SampleSection> {
        vec![
            SampleSection {
                name: "Experience".to_string(),
                bullets: vec![
                    "Led platform migration saving $50k/year".to_string(),
                    "Reduced P99 latency by 40% via caching".to_string(),
                ],
            },
            SampleSection {
                name: "Skills".to_string(),
                bullets: vec!["Rust, Python, TypeScript".to_string()],
            },
            SampleSection {
                name: "Empty Section".to_string(),
                bullets: vec![], // should be skipped
            },
        ]
    }

    #[test]
    fn test_build_contact_line_omits_empty_fields() {
        let profile = sample_profile();
        let line = build_contact_line(&profile);
        // website is empty — should not appear
        assert!(
            !line.contains("faGlobe"),
            "empty website field must not produce icon"
        );
        // email must appear
        assert!(
            line.contains("faEnvelope"),
            "email must produce envelope icon"
        );
        // phone must appear
        assert!(line.contains("faPhone"), "phone must produce phone icon");
    }

    #[test]
    fn test_build_contact_line_escapes_special_chars() {
        let mut profile = sample_profile();
        profile.location = "St. Louis & Chicago".to_string();
        let line = build_contact_line(&profile);
        assert!(
            line.contains(r"\&"),
            "ampersand in location must be escaped"
        );
    }

    #[test]
    fn test_build_sections_latex_skips_empty() {
        let sections = sample_sections();
        let latex = build_sections_latex(&sections);
        // Empty Section has no bullets — must not appear in output
        assert!(
            !latex.contains("Empty Section"),
            "sections with no bullets must be omitted"
        );
        // Non-empty sections must appear
        assert!(
            latex.contains("Experience"),
            "Experience section must appear"
        );
        assert!(latex.contains("Skills"), "Skills section must appear");
    }

    #[test]
    fn test_build_sections_latex_escapes_dollars() {
        let sections = vec![SampleSection {
            name: "Wins".to_string(),
            bullets: vec!["Saved $50k".to_string()],
        }];
        let latex = build_sections_latex(&sections);
        assert!(
            latex.contains(r"\$50k"),
            "dollar sign in bullet must be LaTeX-escaped"
        );
    }

    #[test]
    fn test_render_file_template_replaces_name() {
        // Minimal fake template to test substitution without a real .tex file
        let template = LoadedTemplate {
            metadata: TemplateMetadata {
                id: "test".to_string(),
                name: "Test".to_string(),
                description: "".to_string(),
                tags: vec![],
                engine: "pdflatex".to_string(),
                thumbnail_s3_key: "thumbnails/test.png".to_string(),
            },
            latex_source: "Name: {{FULL_NAME}}\n{{CONTACT_LINE}}\n{{SECTIONS}}".to_string(),
            sample_data: SampleData {
                profile: ProfileData::default(),
                sections: vec![],
            },
        };

        let profile = sample_profile();
        let sections = sample_sections();
        let result = render_file_template(&template, &profile, &sections);

        assert!(
            result.contains("Jane Doe"),
            "full name must appear in output"
        );
        assert!(
            !result.contains("{{FULL_NAME}}"),
            "placeholder must be replaced"
        );
        assert!(
            !result.contains("{{CONTACT_LINE}}"),
            "placeholder must be replaced"
        );
        assert!(
            !result.contains("{{SECTIONS}}"),
            "placeholder must be replaced"
        );
    }

    #[test]
    fn test_load_templates_from_dir_missing_dir_returns_empty() {
        let path = Path::new("/nonexistent/templates/dir");
        let result = load_templates_from_dir(path).unwrap();
        assert!(result.is_empty(), "missing directory must return empty map");
    }

    #[test]
    fn test_load_templates_from_dir_loads_generic_cv() {
        // Use the actual template directory included in the repository
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let templates_dir = repo_root.join("apps/api/templates");

        if !templates_dir.exists() {
            // Template directory not present in this test environment — skip
            return;
        }

        let templates = load_templates_from_dir(&templates_dir).unwrap();
        assert!(
            templates.contains_key("generic-cv"),
            "generic-cv template must be present in templates directory"
        );

        let t = &templates["generic-cv"];
        assert_eq!(t.metadata.engine, "pdflatex");
        assert!(
            !t.latex_source.is_empty(),
            "template.tex must have non-empty LaTeX source"
        );
        assert!(
            t.latex_source.contains(r"\documentclass"),
            "template.tex must contain a \\documentclass declaration"
        );
    }
}
