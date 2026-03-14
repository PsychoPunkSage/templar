#![allow(dead_code)]
//! Background render worker: Redis BRPOP → DB fetch → LaTeX → pdflatex → S3 upload.
//!
//! Queue pattern: LPUSH on enqueue, BRPOP with 5-second timeout on dequeue.
//! Redis key stores `job_id` as a UUID string; worker fetches all data from DB.
//!
//! The worker NEVER panics on individual job failure — it logs the error and
//! continues processing the next job.

use std::collections::HashMap;
use std::sync::Arc;

use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::Client as S3Client;
use redis::AsyncCommands;
use sqlx::PgPool;
use tokio::sync::Semaphore;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::layout::{default_page_config, FontFamily};
use crate::models::resume::{ResumeBulletRow, ResumeRow};
use crate::render::pdflatex::compile_latex;
use crate::render::templates::build_latex_document;
use crate::render::types::{RenderError, RenderParams, ResumeSection};
use crate::templates::{ProfileData, SampleSection, TemplateCache};

/// Redis list key used for the render job queue.
pub const RENDER_QUEUE_KEY: &str = "render:jobs";

// ────────────────────────────────────────────────────────────────────────────
// Worker spawn
// ────────────────────────────────────────────────────────────────────────────

/// Spawns the background render worker as a detached tokio task.
///
/// The worker processes render jobs from the Redis queue indefinitely.
/// It receives clones of all needed resources (Redis, DB, S3) before
/// `AppState` is moved into the Axum router.
///
/// `template_cache` is passed so the worker can look up file-based templates
/// when `resume.template_id` is set. The Arc ensures the cache is shared without
/// copying the HashMap — the worker holds a read lock only during template lookup.
pub fn spawn_render_worker(
    redis: redis::Client,
    db: PgPool,
    s3: S3Client,
    s3_bucket: String,
    template_cache: Arc<TemplateCache>,
) {
    tokio::spawn(async move {
        worker_loop(redis, db, s3, s3_bucket, template_cache).await;
    });
}

// ────────────────────────────────────────────────────────────────────────────
// Worker loop
// ────────────────────────────────────────────────────────────────────────────

/// How many render jobs can run in parallel.
///
/// pdflatex is CPU + I/O bound (compile + S3 upload). On a typical container with
/// 2 vCPUs, 4 concurrent jobs keeps both cores busy without starving the HTTP server.
/// Increase this if containers have more CPU; decrease if you see OOM.
const WORKER_CONCURRENCY: usize = 4;

/// Main worker loop — runs indefinitely, BRPOP-ing jobs from Redis.
///
/// Concurrency model: we allow up to WORKER_CONCURRENCY jobs to run in parallel.
/// Each dequeued job is spawned as an independent tokio task. A semaphore provides
/// back-pressure: the BRPOP loop blocks when all slots are occupied, so Redis doesn't
/// accumulate jobs faster than we can process them.
///
/// Why a semaphore instead of a thread pool?
/// pdflatex compilation is `async` (tokio::process::Command), so it fits naturally
/// on the tokio runtime. spawn() lets jobs overlap their I/O-wait phases (S3 upload,
/// DB queries) without blocking each other. The semaphore caps peak concurrency so
/// we never overload the container.
async fn worker_loop(
    redis: redis::Client,
    db: PgPool,
    s3: S3Client,
    s3_bucket: String,
    template_cache: Arc<TemplateCache>,
) {
    info!("Render worker loop started (concurrency: {WORKER_CONCURRENCY})");

    // Semaphore that limits concurrent render jobs. Arc so it can be cloned into
    // each spawned task — the task holds the permit for its entire lifetime, and
    // the permit is released (slot freed) when the task completes or panics.
    let semaphore = Arc::new(Semaphore::new(WORKER_CONCURRENCY));

    loop {
        let mut conn = match redis.get_multiplexed_async_connection().await {
            Ok(c) => c,
            Err(e) => {
                error!("Render worker: Redis connection failed: {e} — retrying in 5s");
                tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
                continue;
            }
        };

        // Block here if all WORKER_CONCURRENCY slots are in use.
        // This prevents dequeuing more work than we can handle — if 4 jobs are
        // already compiling, we wait until one finishes before pulling the next.
        // acquire_owned() gives us an OwnedSemaphorePermit that can be moved into the task.
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .expect("semaphore closed — this never happens");

        // BRPOP with 5.0-second timeout — returns None on timeout, Some((key, value)) on job
        let result: Result<Option<(String, String)>, redis::RedisError> =
            conn.brpop(RENDER_QUEUE_KEY, 5.0).await;

        match result {
            Ok(None) => {
                // No job arrived within 5 seconds — drop the permit immediately so we
                // don't hold a slot while the queue is empty, then loop.
                drop(permit);
            }
            Ok(Some((_key, job_id_str))) => match Uuid::parse_str(&job_id_str) {
                Ok(job_id) => {
                    info!("Render worker: dequeued job {}", job_id);

                    // Clone the shared resources so the spawned task owns its own handles.
                    // These are all cheap reference-counted clones (Arc / connection pool).
                    let (db2, s3_2, bucket2, tc2) = (
                        db.clone(),
                        s3.clone(),
                        s3_bucket.clone(),
                        Arc::clone(&template_cache),
                    );

                    // Spawn the job as an independent async task so it runs concurrently
                    // with the next BRPOP. The permit is moved into the task and dropped
                    // when process_render_job() returns, freeing the semaphore slot.
                    tokio::spawn(async move {
                        let _permit = permit; // Holds the slot for the lifetime of this job
                        process_render_job(job_id, &db2, &s3_2, &bucket2, &tc2).await;
                    });
                }
                Err(e) => {
                    drop(permit); // Release slot — we're not starting a job
                    error!("Render worker: invalid UUID in queue '{}': {e}", job_id_str);
                }
            },
            Err(e) => {
                drop(permit); // Release slot on error — no job to process
                error!("Render worker: BRPOP error: {e} — reconnecting");
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Job processing
// ────────────────────────────────────────────────────────────────────────────

/// Processes a single render job end-to-end.
///
/// Steps:
/// 1. Mark job 'processing' in render_jobs
/// 2. Fetch resume_id from render_jobs
/// 3. Fetch resume row + bullets from DB (grouped by section)
/// 4. Build LaTeX — file-based template OR legacy font-based path
/// 5. Compile LaTeX via pdflatex (-halt-on-error; packages are on-disk TeX Live files)
/// 6. Upload PDF to S3 (key: pdfs/{resume_id}.pdf)
/// 7. UPDATE resumes: s3_pdf_key + latex_source + status='rendered'
/// 8. Mark job 'done'
///
/// This function is an outer wrapper that catches ALL errors (including those
/// from the early resume_id lookup) and ensures the job is always marked with
/// a terminal status ('done' or 'failed'). A job MUST NEVER be left at
/// 'processing' — that would cause the frontend to poll forever.
///
/// Semaphore note: the caller holds an OwnedSemaphorePermit for the lifetime
/// of this task. If this function panics, tokio drops the future and the permit
/// is released automatically (OwnedSemaphorePermit implements Drop). This is
/// safe and intentional — no explicit guard is needed, but the permit drop is
/// documented here so future engineers understand why there is no manual release.
async fn process_render_job(
    job_id: Uuid,
    db: &PgPool,
    s3: &S3Client,
    s3_bucket: &str,
    template_cache: &Arc<TemplateCache>,
) {
    info!(job_id = %job_id, "Render job dequeued — starting processing");

    // Step 1: Mark job as processing so the status endpoint shows progress.
    // If this fails, we still try the inner work — but log the failure.
    if let Err(e) = update_job_status(db, job_id, "processing", None).await {
        error!(
            job_id = %job_id,
            error = %e,
            "Render worker: failed to mark job as processing — continuing anyway"
        );
    }

    // Step 2: Fetch resume_id for this job. If not found, mark failed immediately.
    // This was previously a silent `return` which left the job at 'processing' forever.
    let resume_id = match sqlx::query_scalar::<_, Uuid>(
        "SELECT resume_id FROM render_jobs WHERE id = $1",
    )
    .bind(job_id)
    .fetch_optional(db)
    .await
    {
        Ok(Some(id)) => {
            info!(job_id = %job_id, resume_id = %id, "Render job: resume_id resolved");
            id
        }
        Ok(None) => {
            error!(job_id = %job_id, "Render job not found in render_jobs — marking failed");
            let _ = update_job_status(
                db,
                job_id,
                "failed",
                Some("Render job row not found in database"),
            )
            .await;
            return;
        }
        Err(e) => {
            error!(job_id = %job_id, error = %e, "DB error fetching render job — marking failed");
            let _ = update_job_status(
                db,
                job_id,
                "failed",
                Some(&format!("DB error fetching job: {e}")),
            )
            .await;
            return;
        }
    };

    // Inner async block returns Result so all errors bubble up to the single
    // error handler below, which marks the job failed with the error message.
    let result: Result<(), RenderError> = async {
        // Step 3: Fetch render data from DB (resume row + bullets grouped by section)
        info!(job_id = %job_id, resume_id = %resume_id, "Render job: fetching render data from DB");
        let (params, resume_template_id) = fetch_render_data(db, resume_id).await?;
        info!(
            job_id = %job_id,
            resume_id = %resume_id,
            section_count = params.sections.len(),
            template_id = ?resume_template_id,
            "Render job: render data fetched"
        );

        // Step 4: Build LaTeX document.
        // Two paths:
        //   a) File-based template (e.g. generic-cv): uses render_file_template()
        //      with the .tex file from disk, substituting profile + bullets.
        //   b) Legacy font-based path: uses build_latex_document() which generates
        //      XeLaTeX with \usepackage{fontspec} + \setmainfont{<font>}.
        // The file-based path is preferred when a template_id is set on the resume.
        let engine_type = if resume_template_id.is_some() {
            "file-template"
        } else {
            "xelatex-legacy"
        };
        info!(
            job_id = %job_id,
            resume_id = %resume_id,
            engine = engine_type,
            "Render job: building LaTeX document"
        );
        let latex =
            build_latex_for_job(&params, resume_template_id.as_deref(), template_cache, db).await;
        info!(
            job_id = %job_id,
            resume_id = %resume_id,
            latex_bytes = latex.len(),
            engine = engine_type,
            "Render job: LaTeX source built"
        );

        // Step 5: Compile LaTeX → PDF via pdflatex.
        // TeX Live packages are installed on disk via apt — no cache hash, no network.
        // compile_latex() runs pdflatex with -halt-on-error so failures are fast and clear.
        info!(
            job_id = %job_id,
            resume_id = %resume_id,
            "Render job: spawning pdflatex"
        );
        let pdflatex_result = compile_latex(&latex, job_id).await.map_err(|e| {
            // Log pdflatex failure with FULL stderr before propagating the error.
            // This is the key diagnostic log — it contains the LaTeX error message
            // that explains why compilation failed (e.g. missing font, bad package).
            if let RenderError::CompilationFailed {
                exit_code,
                ref stderr,
            } = e
            {
                error!(
                    job_id = %job_id,
                    resume_id = %resume_id,
                    exit_code = exit_code,
                    stderr = %stderr,
                    "pdflatex compilation FAILED"
                );
            } else {
                error!(
                    job_id = %job_id,
                    resume_id = %resume_id,
                    error = %e,
                    "pdflatex invocation error"
                );
            }
            e
        })?;

        info!(
            job_id = %job_id,
            resume_id = %resume_id,
            duration_ms = pdflatex_result.duration_ms,
            pdf_bytes = pdflatex_result.pdf_bytes.len(),
            stderr_bytes = pdflatex_result.stderr.len(),
            "pdflatex compilation succeeded"
        );
        // pdflatex writes warnings (e.g. overfull hbox, underfull hbox) to stderr
        // even on success. Log them at DEBUG (not WARN — they are routine and not
        // actionable unless the operator is actively tuning LaTeX spacing).
        if !pdflatex_result.stderr.is_empty() {
            tracing::debug!(
                job_id = %job_id,
                resume_id = %resume_id,
                stderr = %pdflatex_result.stderr,
                "pdflatex stderr (warnings)"
            );
        }

        // Step 6: Upload PDF bytes to S3 (key: pdfs/{resume_id}.pdf)
        let s3_key = format!("pdfs/{resume_id}.pdf");
        info!(
            job_id = %job_id,
            resume_id = %resume_id,
            s3_key = %s3_key,
            byte_count = pdflatex_result.pdf_bytes.len(),
            "Render job: uploading PDF to S3"
        );
        let s3_key = upload_pdf_to_s3(s3, s3_bucket, resume_id, pdflatex_result.pdf_bytes)
            .await
            .map_err(|e| {
                error!(
                    job_id = %job_id,
                    resume_id = %resume_id,
                    error = %e,
                    "S3 upload failed"
                );
                e
            })?;
        info!(
            job_id = %job_id,
            resume_id = %resume_id,
            s3_key = %s3_key,
            "Render job: PDF uploaded to S3"
        );

        // Step 7: Write latex_source + s3_pdf_key + rendered status in one query.
        // Deliberately done AFTER successful compilation: storing latex_source before
        // compile_latex() means a failed compile would leave stale/broken LaTeX in the
        // DB. Combining it with the rendered-status update keeps things consistent —
        // latex_source is only ever set when we know the compiled PDF is valid.
        sqlx::query(
            "UPDATE resumes \
             SET s3_pdf_key = $1, latex_source = $2, status = 'rendered', updated_at = NOW() \
             WHERE id = $3",
        )
        .bind(&s3_key)
        .bind(&latex)
        .bind(resume_id)
        .execute(db)
        .await?;

        // Step 8: Mark job as done — frontend polling will see this and fetch the PDF
        update_job_status(db, job_id, "done", None).await?;

        info!(
            job_id = %job_id,
            resume_id = %resume_id,
            s3_key = %s3_key,
            "Render job DONE"
        );

        Ok(())
    }
    .await;

    if let Err(e) = result {
        error!(
            job_id = %job_id,
            resume_id = %resume_id,
            error = %e,
            "Render job FAILED — marking as failed in DB"
        );
        let _ = update_job_status(db, job_id, "failed", Some(&e.to_string())).await;
    }
}

// ────────────────────────────────────────────────────────────────────────────
// DB helpers
// ────────────────────────────────────────────────────────────────────────────

/// Fetches resume and bullets from DB and constructs RenderParams.
///
/// Returns `(RenderParams, Option<template_id>)`. The template_id is passed
/// to `build_latex_for_job` to decide which LaTeX code path to use.
///
/// Uses default page config (Inter 11pt, 1" margins) — per-user font/margins
/// can be wired later without changing this interface.
async fn fetch_render_data(
    db: &PgPool,
    resume_id: Uuid,
) -> Result<(RenderParams, Option<String>), RenderError> {
    // Fetch resume row — includes template_id (added in migration 004)
    let resume = sqlx::query_as::<_, ResumeRow>("SELECT * FROM resumes WHERE id = $1")
        .bind(resume_id)
        .fetch_optional(db)
        .await?
        .ok_or(RenderError::ResumeNotFound(resume_id))?;

    let resume_template_id = resume.template_id.clone();

    // Fetch all bullets ordered by section + insertion order
    let bullets = sqlx::query_as::<_, ResumeBulletRow>(
        "SELECT * FROM resume_bullets WHERE resume_id = $1 ORDER BY section, id",
    )
    .bind(resume_id)
    .fetch_all(db)
    .await?;

    // Group bullets by section, preserving first-seen section order
    let mut section_order: Vec<String> = Vec::new();
    let mut section_map: HashMap<String, Vec<String>> = HashMap::new();

    for bullet in bullets {
        if !section_map.contains_key(&bullet.section) {
            section_order.push(bullet.section.clone());
        }
        section_map
            .entry(bullet.section)
            .or_default()
            .push(bullet.bullet_text);
    }

    let sections = section_order
        .into_iter()
        .map(|name| {
            let section_bullets = section_map.remove(&name).unwrap_or_default();
            ResumeSection {
                name,
                bullets: section_bullets,
            }
        })
        .collect();

    // Use default page config — per-user font/margins can be wired in later
    let page_config = default_page_config(FontFamily::Inter);

    Ok((
        RenderParams {
            resume_id,
            font: page_config.font,
            font_size_pt: page_config.font_size_pt,
            margin_left_in: page_config.margin_left_in,
            margin_right_in: page_config.margin_right_in,
            sections,
        },
        resume_template_id,
    ))
}

/// Builds the LaTeX document string for a render job.
///
/// Routing logic:
///   - If `template_id` is Some and matches a file-based template in the cache
///     → use `render_file_template()` with the user's profile data
///   - Otherwise (None or unrecognized id) → fall back to `build_latex_document()`
///     which uses the 5 built-in font-based templates
///
/// This fallback is intentional: existing resumes generated before the template
/// system existed (template_id = NULL) will still render correctly.
async fn build_latex_for_job(
    params: &RenderParams,
    template_id: Option<&str>,
    template_cache: &Arc<TemplateCache>,
    db: &PgPool,
) -> String {
    if let Some(tid) = template_id {
        // Acquire read lock — lightweight, no contention in practice
        let cache = template_cache.read().await;
        if let Some(template) = cache.get(tid) {
            // Fetch user profile from context_entries to populate the header.
            // If the user has no profile entry, use empty defaults — the PDF will
            // have a blank header, which is still a valid document. The user can
            // fix this by adding a profile entry via the /context page.
            let profile = fetch_user_profile(db, params.resume_id)
                .await
                .unwrap_or_else(|e| {
                    warn!(
                        "No profile entry for resume {} (template '{}'), using defaults: {}",
                        params.resume_id, tid, e
                    );
                    ProfileData::default()
                });

            // Convert RenderParams sections → SampleSection (same shape, different module)
            let sections: Vec<SampleSection> = params
                .sections
                .iter()
                .map(|s| SampleSection {
                    name: s.name.clone(),
                    bullets: s.bullets.clone(),
                })
                .collect();

            return crate::templates::render_file_template(template, &profile, &sections);
        } else {
            // template_id set but not in cache — log and fall through to legacy path
            warn!(
                "Resume {} has template_id '{}' but it's not in the cache — using legacy template",
                params.resume_id, tid
            );
        }
    }

    // None branch: try generic-cv from cache first, then fall back to minimal pdflatex.
    // NEVER call build_latex_document() here — it emits XeLaTeX (fontspec) which
    // requires XeLaTeX/LuaLaTeX and specific fonts. The None branch targets plain
    // pdflatex so it compiles without any font installation.
    {
        let cache = template_cache.read().await;
        if let Some(template) = cache.get("generic-cv") {
            let profile = fetch_user_profile(db, params.resume_id)
                .await
                .unwrap_or_else(|e| {
                    warn!(
                        "No profile entry for resume {} (generic-cv fallback), using defaults: {}",
                        params.resume_id, e
                    );
                    ProfileData::default()
                });

            let sections: Vec<SampleSection> = params
                .sections
                .iter()
                .map(|s| SampleSection {
                    name: s.name.clone(),
                    bullets: s.bullets.clone(),
                })
                .collect();

            return crate::templates::render_file_template(template, &profile, &sections);
        }
    }

    // Final fallback: minimal pdflatex document — no extra fonts required.
    build_minimal_pdflatex_document(params)
}

/// Builds a minimal pdflatex-compatible document from RenderParams.
///
/// Uses only standard packages available in any TeX Live installation
/// (lmodern, T1 fontenc, geometry, enumitem, titlesec, xcolor, hyperref).
/// No fontspec, no XeLaTeX-only packages — this compiles cleanly with plain pdflatex.
///
/// Called when: template_id is None AND generic-cv is not in the template cache.
fn build_minimal_pdflatex_document(params: &RenderParams) -> String {
    use crate::render::templates::escape_latex;

    let mut doc = String::with_capacity(4096);

    doc.push_str(
        r#"\documentclass[letterpaper,11pt]{article}
\usepackage{lmodern}
\usepackage[T1]{fontenc}
\usepackage[utf8]{inputenc}
\usepackage[margin=0.75in,top=0.6in,bottom=0.6in]{geometry}
\usepackage{titlesec}
\usepackage{enumitem}
\usepackage[dvipsnames]{xcolor}
\usepackage[hidelinks]{hyperref}
\definecolor{headerblue}{HTML}{003366}
\titleformat{\section}{\color{headerblue}\large\bfseries}{}{0em}{}[\color{headerblue}\titlerule]
\titlespacing*{\section}{0pt}{6pt}{4pt}
\setlist[itemize]{leftmargin=1.2em,itemsep=1pt,parsep=0pt,topsep=2pt}
\pagestyle{empty}
\begin{document}
"#,
    );

    for section in &params.sections {
        doc.push_str(&format!(
            "\n\\section{{{}}}\n\\begin{{itemize}}\n",
            escape_latex(&section.name)
        ));
        for bullet in &section.bullets {
            doc.push_str(&format!("  \\item {}\n", escape_latex(bullet)));
        }
        doc.push_str("\\end{itemize}\n");
    }

    doc.push_str("\n\\end{document}\n");
    doc
}

/// Fetches the user's profile data from their context entries.
///
/// Queries `context_entries` for the most recent entry with `entry_type = 'profile'`
/// and attempts to deserialize common fields (full_name, email, phone, location,
/// linkedin, website) from the JSONB `data` column.
///
/// If the user has no profile entry, returns an error (caller uses ProfileData::default()).
async fn fetch_user_profile(db: &PgPool, resume_id: Uuid) -> anyhow::Result<ProfileData> {
    // First get user_id from the resume
    let user_id = sqlx::query_scalar::<_, Uuid>("SELECT user_id FROM resumes WHERE id = $1")
        .bind(resume_id)
        .fetch_optional(db)
        .await?
        .ok_or_else(|| anyhow::anyhow!("resume not found"))?;

    // Get the most recent profile context entry
    let data: Option<serde_json::Value> = sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT data FROM context_entries
           WHERE user_id = $1 AND entry_type = 'profile'
           ORDER BY version DESC
           LIMIT 1"#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await?;

    let data = data.ok_or_else(|| anyhow::anyhow!("no profile entry for user {}", user_id))?;

    // Extract fields leniently — missing keys become empty strings
    let get_str = |key: &str| -> String {
        data.get(key)
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    };

    Ok(ProfileData {
        full_name: get_str("full_name"),
        email: get_str("email"),
        phone: get_str("phone"),
        location: get_str("location"),
        linkedin: get_str("linkedin"),
        website: get_str("website"),
    })
}

/// Updates `render_jobs.status` (and optionally `error_message`) for a given job.
async fn update_job_status(
    db: &PgPool,
    job_id: Uuid,
    status: &str,
    error_msg: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE render_jobs SET status = $1, error_message = $2, updated_at = NOW() WHERE id = $3",
    )
    .bind(status)
    .bind(error_msg)
    .bind(job_id)
    .execute(db)
    .await?;
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// S3 upload
// ────────────────────────────────────────────────────────────────────────────

/// Uploads PDF bytes to S3 and returns the object key.
///
/// Key pattern: `pdfs/{resume_id}.pdf`
async fn upload_pdf_to_s3(
    s3: &S3Client,
    s3_bucket: &str,
    resume_id: Uuid,
    pdf_bytes: Vec<u8>,
) -> Result<String, RenderError> {
    let key = format!("pdfs/{resume_id}.pdf");

    s3.put_object()
        .bucket(s3_bucket)
        .key(&key)
        .body(ByteStream::from(pdf_bytes))
        .content_type("application/pdf")
        .send()
        .await
        .map_err(|e| RenderError::S3Upload(e.to_string()))?;

    Ok(key)
}

// ────────────────────────────────────────────────────────────────────────────
// Sync enqueue (called from generation/generator.rs via spawn_blocking)
// ────────────────────────────────────────────────────────────────────────────

/// Enqueues a render job by pushing the job_id UUID string to the Redis list.
///
/// This is a synchronous function, intended to be called via
/// `tokio::task::spawn_blocking` from async contexts.
pub fn enqueue_render_job_sync(redis: &redis::Client, job_id: Uuid) -> Result<(), RenderError> {
    use redis::Commands;
    let mut conn = redis.get_connection()?;
    conn.lpush::<_, _, ()>(RENDER_QUEUE_KEY, job_id.to_string())?;
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Integration test — requires a live Redis instance.
    #[tokio::test]
    #[ignore]
    async fn test_enqueue_render_job_sync_pushes_to_list() {
        let redis_url =
            std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let client = redis::Client::open(redis_url).expect("Redis client");
        let job_id = Uuid::new_v4();

        enqueue_render_job_sync(&client, job_id).expect("enqueue should succeed");

        // Verify the job_id was pushed to the queue
        let mut conn = client
            .get_multiplexed_async_connection()
            .await
            .expect("async conn");
        let popped: Option<(String, String)> = conn.brpop(RENDER_QUEUE_KEY, 1.0).await.unwrap();
        let (_key, value) = popped.expect("job should be in queue");
        assert_eq!(value, job_id.to_string());
    }

    /// Integration test — requires a live PostgreSQL instance.
    #[tokio::test]
    #[ignore]
    async fn test_fetch_render_data_returns_not_found() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&db_url)
            .await
            .expect("DB pool");

        let fake_id = Uuid::new_v4();
        let result = fetch_render_data(&pool, fake_id).await;
        assert!(
            matches!(result, Err(RenderError::ResumeNotFound(_))),
            "should return ResumeNotFound for unknown resume, got: {:?}",
            result
        );
    }

    /// Integration test — requires live PostgreSQL.
    #[tokio::test]
    #[ignore]
    async fn test_process_render_job_marks_failed_on_missing_resume() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&db_url)
            .await
            .expect("DB pool");

        // This test would need a real render_jobs row and a fake S3 client.
        // Marked ignore — covered by integration test suite.
        let _ = pool;
    }

    /// Integration test — requires live PostgreSQL.
    #[tokio::test]
    #[ignore]
    async fn test_update_job_status_all_transitions() {
        let db_url = std::env::var("DATABASE_URL").expect("DATABASE_URL required");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect(&db_url)
            .await
            .expect("DB pool");

        // Would insert a render_jobs row, then cycle through status transitions.
        // Marked ignore — covered by integration test suite.
        let _ = pool;
    }
}
