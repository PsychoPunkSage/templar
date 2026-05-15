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
use crate::render::types::{RenderError, RenderParams, ResumeSection, ResumeSubEntry};
use crate::templates::{ProfileData, SampleSection, SampleSubEntry, TemplateCache};

/// Redis list key used for the render job queue.
pub const RENDER_QUEUE_KEY: &str = "render:jobs";

/// (source_entry_id, page_number, entry_header_latex, bullet_texts)
type SectionEntry = (Uuid, i16, Option<String>, Vec<String>);

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

/// Main worker loop — runs indefinitely, BRPOP-ing jobs from Redis.
///
/// Concurrency model: we allow up to RENDER_WORKER_COUNT (env, default 4) jobs to
/// run in parallel. Each dequeued job is spawned as an independent tokio task. A
/// semaphore provides back-pressure: the BRPOP loop blocks when all slots are
/// occupied, so Redis doesn't accumulate jobs faster than we can process them.
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
    let concurrency: usize = std::env::var("RENDER_WORKER_COUNT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(4);

    info!("Render worker loop started (concurrency: {concurrency})");

    // Semaphore that limits concurrent render jobs. Arc so it can be cloned into
    // each spawned task — the task holds the permit for its entire lifetime, and
    // the permit is released (slot freed) when the task completes or panics.
    let semaphore = Arc::new(Semaphore::new(concurrency));

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

                    // Record queue depth after dequeue (best-effort — never block on error)
                    if let Ok(mut depth_conn) = redis.get_multiplexed_async_connection().await {
                        if let Ok(depth) = depth_conn.llen::<_, i64>(RENDER_QUEUE_KEY).await {
                            crate::metrics::set_render_queue_depth(depth);
                        }
                    }

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
                        let start = std::time::Instant::now();
                        let label =
                            match process_render_job(job_id, &db2, &s3_2, &bucket2, &tc2).await {
                                Ok(()) => "success",
                                Err(_) => "failed",
                            };
                        crate::metrics::observe_render_duration(
                            start.elapsed().as_secs_f64(),
                            label,
                        );
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
) -> Result<(), RenderError> {
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
            return Err(RenderError::ResumeNotFound(job_id));
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
            return Err(RenderError::Database(e));
        }
    };

    // Inner async block returns Result so all errors bubble up to the single
    // error handler below, which marks the job failed with the error message.
    let result: Result<(), RenderError> = async {
        // Step 3: Fetch render data from DB (resume row + bullets grouped by section)
        info!(job_id = %job_id, resume_id = %resume_id, "Render job: fetching render data from DB");
        let (mut params, resume_template_id, resume_type) =
            fetch_render_data(db, resume_id, template_cache).await?;
        info!(
            job_id = %job_id,
            resume_id = %resume_id,
            section_count = params.sections.len(),
            template_id = ?resume_template_id,
            "Render job: render data fetched"
        );

        // Fetch profile once — used for hash computation AND passed into build_latex_for_job.
        // This avoids a double DB fetch that the old code performed (once in build_latex_for_job,
        // then again if falling through to the generic-cv branch).
        let profile = fetch_user_profile(db, resume_id, None)
            .await
            .unwrap_or_else(|e| {
                warn!(
                    job_id = %job_id,
                    resume_id = %resume_id,
                    error = %e,
                    "No profile for resume — using defaults"
                );
                ProfileData::default()
            });

        // Compute a content hash covering template + profile + sections.
        // If the hash matches the stored value AND a PDF already exists in S3,
        // we skip compilation entirely — the rendered PDF is still valid.
        let new_hash =
            compute_render_hash(resume_template_id.as_deref(), &profile, &params.sections);

        // Cache-hit check: query current hash + pdf key from DB
        let cached: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT content_hash, s3_pdf_key FROM resumes WHERE id = $1")
                .bind(resume_id)
                .fetch_optional(db)
                .await
                .unwrap_or(None);

        if let Some((Some(existing_hash), Some(_pdf_key))) = cached {
            if existing_hash == new_hash {
                info!(
                    job_id = %job_id,
                    resume_id = %resume_id,
                    "Content hash unchanged — skipping render, reusing existing PDF"
                );
                update_job_status(db, job_id, "done", None).await?;
                return Ok(());
            }
        }

        // Step 4 + 5: Build LaTeX and compile to PDF.
        // For single-page resumes, add a post-render overflow guard: if pdflatex produces
        // more than 1 page, drop the lowest-scoring sub-entry and retry (up to
        // MAX_POST_RENDER_RETRIES times). CV mode trusts the paginator and is skipped.
        //
        // build_latex_for_job consumes `profile` by value, so we pre-clone it.
        let max_post_render_retries: u8 = std::env::var("MAX_POST_RENDER_RETRIES")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(2);
        let target_pages: u8 = if resume_type == "cv" { 0 } else { 1 }; // 0 = skip check for CV

        // Step 4 + 5: Build LaTeX and compile to PDF (with post-render overflow retry).
        // For single-page resumes, if pdflatex produces > 1 page, drop the last sub-entry
        // from `params` and retry (up to max_post_render_retries times).
        // CV mode trusts the paginator and skips the page-count check.
        let mut post_render_attempt: u8 = 0;
        let pdflatex_result;
        let latex_source; // declared outside loop so Step 7 can bind it to DB
        loop {
            // Step 4: Build LaTeX document.
            // Unified path: always pdflatex via file-based template.
            //   a) template_id set + in cache → render_file_template()
            //   b) template_id set but not in cache → warn, try generic-cv
            //   c) generic-cv in cache → render_file_template() with generic-cv
            //   d) nothing in cache → build_minimal_pdflatex_document()
            // build_latex_for_job consumes `profile` by value — clone for each attempt.
            let this_latex_source = build_latex_for_job(
                &params,
                resume_template_id.as_deref(),
                template_cache,
                profile.clone(),
                &resume_type,
            )
            .await;
            info!(
                job_id = %job_id,
                resume_id = %resume_id,
                latex_bytes = this_latex_source.len(),
                "Render job: LaTeX source built"
            );

            // Step 5: Compile LaTeX → PDF via pdflatex (unified; no xelatex path).
            info!(
                job_id = %job_id,
                resume_id = %resume_id,
                "Render job: spawning pdflatex compiler"
            );
            let compiled = compile_latex(&this_latex_source, job_id)
                .await
                .map_err(|e| {
                    // Log failure with FULL stderr before propagating the error.
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
                            "LaTeX compilation FAILED"
                        );
                    } else {
                        error!(
                            job_id = %job_id,
                            resume_id = %resume_id,
                            error = %e,
                            "LaTeX invocation error"
                        );
                    }
                    e
                })?;

            info!(
                job_id = %job_id,
                resume_id = %resume_id,
                duration_ms = compiled.duration_ms,
                pdf_bytes = compiled.pdf_bytes.len(),
                stderr_bytes = compiled.stderr.len(),
                actual_page_count = ?compiled.actual_page_count,
                "pdflatex compilation succeeded"
            );
            // pdflatex writes warnings (e.g. overfull hbox, underfull hbox) to stderr
            // even on success. Log them at DEBUG (not WARN — they are routine and not
            // actionable unless the operator is actively tuning LaTeX spacing).
            if !compiled.stderr.is_empty() {
                tracing::debug!(
                    job_id = %job_id,
                    resume_id = %resume_id,
                    stderr = %compiled.stderr,
                    "pdflatex stderr (warnings)"
                );
            }

            // Post-render overflow check (single-page only).
            // If lopdf reports > 1 page AND we have retries left, drop the last
            // sub-entry and compile again. This is a final safety net — the layout
            // simulator should have caught this, but font-metric approximation means
            // edge cases can slip through.
            if target_pages > 0 {
                if let Some(page_count) = compiled.actual_page_count {
                    if page_count > target_pages && post_render_attempt < max_post_render_retries {
                        post_render_attempt += 1;
                        warn!(
                            job_id = %job_id,
                            resume_id = %resume_id,
                            page_count = page_count,
                            target_pages = target_pages,
                            attempt = post_render_attempt,
                            "PDF exceeds target page count — dropping last sub-entry and retrying"
                        );
                        drop_last_sub_entry(&mut params);
                        continue; // Re-run build_latex_for_job + compile_latex
                    } else if page_count > target_pages {
                        warn!(
                            job_id = %job_id,
                            resume_id = %resume_id,
                            page_count = page_count,
                            attempts = post_render_attempt,
                            "PDF still exceeds target after max retries — accepting as-is"
                        );
                    }
                }
            }

            pdflatex_result = compiled;
            latex_source = this_latex_source;
            break;
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

        // Step 7: Write latex_source + s3_pdf_key + content_hash + rendered status.
        // Deliberately done AFTER successful compilation: storing latex_source before
        // compile_latex() means a failed compile would leave stale/broken LaTeX in the
        // DB. Combining it with the rendered-status update keeps things consistent —
        // latex_source and content_hash are only ever set when we know the PDF is valid.
        sqlx::query(
            "UPDATE resumes \
             SET s3_pdf_key = $1, latex_source = $2, content_hash = $3, \
                 status = 'rendered', updated_at = NOW() \
             WHERE id = $4",
        )
        .bind(&s3_key)
        .bind(&latex_source)
        .bind(&new_hash)
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

    if let Err(ref e) = result {
        error!(
            job_id = %job_id,
            resume_id = %resume_id,
            error = %e,
            "Render job FAILED — marking as failed in DB"
        );
        let _ = update_job_status(db, job_id, "failed", Some(&e.to_string())).await;
    }

    result
}

// ────────────────────────────────────────────────────────────────────────────
// DB helpers
// ────────────────────────────────────────────────────────────────────────────

/// Fetches resume and bullets from DB and constructs RenderParams.
///
/// Returns `(RenderParams, Option<template_id>)`. The template_id is passed
/// to `build_latex_for_job` to decide which LaTeX code path to use.
///
/// Derives PageConfig from the template's declared layout physics when a template
/// is available; falls back to default Inter 11pt, 1" margins otherwise.
/// Returns `(RenderParams, resume_template_id, resume_type)`.
async fn fetch_render_data(
    db: &PgPool,
    resume_id: Uuid,
    template_cache: &Arc<TemplateCache>,
) -> Result<(RenderParams, Option<String>, String), RenderError> {
    // Fetch resume row — includes template_id (added in migration 004)
    let resume = sqlx::query_as::<_, ResumeRow>("SELECT * FROM resumes WHERE id = $1")
        .bind(resume_id)
        .fetch_optional(db)
        .await?
        .ok_or(RenderError::ResumeNotFound(resume_id))?;

    let resume_template_id = resume.template_id.clone();
    let resume_type = if resume.resume_type.is_empty() {
        "single_page".to_string()
    } else {
        resume.resume_type.clone()
    };

    // Fetch bullets ordered: page_number ASC, section ASC, order_idx ASC, id ASC
    // page_number=1 for all single-page resumes (added migration 016, default 1).
    // order_idx preserves the relevance-ranked insertion order from the generation pipeline.
    let bullets = sqlx::query_as::<_, ResumeBulletRow>(
        "SELECT * FROM resume_bullets WHERE resume_id = $1 ORDER BY page_number, section, order_idx, id",
    )
    .bind(resume_id)
    .fetch_all(db)
    .await?;

    // Group: section_name → ordered list of (source_entry_id, page_number, entry_header, Vec<bullet_text>)
    // Preserve first-seen ordering of sections and entries within sections.
    // page_number comes from the bullet row — for multi-page CV resumes, bullets on different
    // pages get different page_number values. For single-page resumes, all are 1.
    let mut section_order: Vec<String> = Vec::new();
    // section → Vec<(source_entry_id, page_number, entry_header, Vec<bullet_text>)>
    let mut section_entries: HashMap<String, Vec<SectionEntry>> = HashMap::new();

    for bullet in &bullets {
        if !section_entries.contains_key(&bullet.section) {
            section_order.push(bullet.section.clone());
            section_entries.insert(bullet.section.clone(), Vec::new());
        }

        let entries = section_entries.get_mut(&bullet.section).unwrap();

        // Find existing entry group or create new one
        if let Some(group) = entries
            .iter_mut()
            .find(|(id, _, _, _)| *id == bullet.source_entry_id)
        {
            // Add bullet to existing group (skip empty placeholder for skills entries)
            if !bullet.bullet_text.is_empty() {
                group.3.push(bullet.bullet_text.clone());
            }
        } else {
            // New entry group: take entry_header from this bullet (it's the first in the group)
            let initial_bullets = if bullet.bullet_text.is_empty() {
                vec![]
            } else {
                vec![bullet.bullet_text.clone()]
            };
            // page_number defaults to 1 if column absent (pre-migration 016 resumes)
            let page_num = bullet.page_number.max(1);
            entries.push((
                bullet.source_entry_id,
                page_num,
                bullet.entry_header.clone(),
                initial_bullets,
            ));
        }
    }

    // Build ResumeSection with ResumeSubEntry (page_number preserved for CV mode)
    let sections: Vec<ResumeSection> = section_order
        .into_iter()
        .map(|section_name| {
            let entries = section_entries.remove(&section_name).unwrap_or_default();
            let sub_entries = entries
                .into_iter()
                .map(
                    |(_, page_number, header_latex, entry_bullets)| ResumeSubEntry {
                        header_latex,
                        bullets: entry_bullets,
                        page_number,
                    },
                )
                .collect();
            ResumeSection {
                name: section_name,
                sub_entries,
            }
        })
        .collect();

    // Derive page config from the template's declared layout physics when available;
    // fall back to default Inter 11pt, 1" margins for resumes without a template.
    let page_config = resume_template_id
        .as_deref()
        .and_then(|id| {
            template_cache
                .try_read()
                .ok()
                .and_then(|cache| cache.get(id).map(|t| t.metadata.page_config()))
        })
        .unwrap_or_else(|| {
            crate::layout::font_metrics::default_page_config(
                crate::layout::font_metrics::FontFamily::Inter,
            )
        });

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
        resume_type,
    ))
}

/// Computes a deterministic SHA-256 hash of all render inputs.
///
/// The hash covers: template_id, profile fields (name/email/phone/location),
/// and all section names + bullets (sorted so insertion order doesn't affect the hash).
///
/// Used for cache-hit detection: if the hash is unchanged from the stored value
/// AND an S3 key already exists, we can skip re-rendering entirely.
fn compute_render_hash(
    template_id: Option<&str>,
    profile: &ProfileData,
    sections: &[ResumeSection],
) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(template_id.unwrap_or("").as_bytes());
    hasher.update(b":");
    hasher.update(profile.full_name.as_bytes());
    hasher.update(b":");
    hasher.update(profile.email.as_bytes());
    hasher.update(b":");
    hasher.update(profile.phone.as_bytes());
    hasher.update(b":");
    hasher.update(profile.location.as_bytes());
    // Sort sections by name so the hash is stable regardless of section insertion order
    let mut sorted_sections: Vec<_> = sections.iter().collect();
    sorted_sections.sort_by(|a, b| a.name.cmp(&b.name));
    for s in &sorted_sections {
        hasher.update(b"::");
        hasher.update(s.name.as_bytes());
        let mut bullets: Vec<&str> = s.all_bullets();
        bullets.sort();
        for b_text in &bullets {
            hasher.update(b":");
            hasher.update(b_text.as_bytes());
        }
    }
    hex::encode(hasher.finalize())
}

/// Groups `ResumeSection`s by page number for multi-page CV rendering.
///
/// Returns a `Vec<Vec<SampleSection>>` where the outer index is `page_number - 1`.
/// Sub-entries from different pages of the same logical section are split across
/// the page groups — section headers repeat on each page they appear (since
/// the paginator already cleared seen_sections on page advance).
///
/// `resume_sections` uses `ResumeSubEntry` (which has `page_number`).
/// The output `SampleSection`s carry NO page_number — they're rendered within one page.
fn group_sections_by_page(resume_sections: &[ResumeSection]) -> Vec<Vec<SampleSection>> {
    // Determine the maximum page number
    let max_page = resume_sections
        .iter()
        .flat_map(|s| s.sub_entries.iter())
        .map(|se| se.page_number as usize)
        .max()
        .unwrap_or(1)
        .max(1);

    let mut pages: Vec<Vec<SampleSection>> = vec![Vec::new(); max_page];

    for section in resume_sections {
        // Group sub-entries by page
        let mut page_sub_entries: std::collections::HashMap<i16, Vec<SampleSubEntry>> =
            std::collections::HashMap::new();
        for se in &section.sub_entries {
            let page_num = se.page_number.max(1);
            page_sub_entries
                .entry(page_num)
                .or_default()
                .push(SampleSubEntry {
                    header_latex: se.header_latex.clone(),
                    bullets: se.bullets.clone(),
                });
        }

        for (page_num, sub_entries) in page_sub_entries {
            let page_idx = (page_num as usize).saturating_sub(1).min(max_page - 1);
            pages[page_idx].push(SampleSection {
                name: section.name.clone(),
                sub_entries,
            });
        }
    }

    // Remove trailing empty pages
    while pages.last().map(|p| p.is_empty()).unwrap_or(false) {
        pages.pop();
    }
    if pages.is_empty() {
        pages.push(Vec::new());
    }

    pages
}

/// Builds the LaTeX document string for a render job.
///
/// Routing logic:
///   - If `template_id` is Some and matches a file-based template in the cache
///     → use `render_file_template()` with the user's profile data
///   - Otherwise (None or unrecognized id) → try generic-cv from cache,
///     then fall back to `build_minimal_pdflatex_document()`
///
/// For CV mode (`resume_type == "cv"`), uses `build_paginated_sections_latex`
/// which emits `\newpage` between pages.
///
/// `profile` is passed in (already fetched by the caller) to avoid a double DB fetch.
async fn build_latex_for_job(
    params: &RenderParams,
    template_id: Option<&str>,
    template_cache: &Arc<TemplateCache>,
    profile: ProfileData,
    resume_type: &str,
) -> String {
    // Apply canonical section ordering — done once here, reused in both template branches
    let ordered_sections = crate::render::section_order::order_sections(&params.sections);
    let sections: Vec<SampleSection> = ordered_sections
        .iter()
        .map(|s| SampleSection {
            name: s.name.clone(),
            sub_entries: s
                .sub_entries
                .iter()
                .map(|se| SampleSubEntry {
                    header_latex: se.header_latex.clone(),
                    bullets: se.bullets.clone(),
                })
                .collect(),
        })
        .collect();

    // Determine if this is a real multi-page document (page_number > 1 exists in data).
    let is_multipage_cv = resume_type == "cv"
        && params
            .sections
            .iter()
            .any(|s| s.sub_entries.iter().any(|se| se.page_number > 1));

    // Helper: renders the template with either paginated or flat sections.
    let render_with_template = |template: &crate::templates::LoadedTemplate| -> String {
        if is_multipage_cv {
            let fmt = template
                .metadata
                .section_formatting
                .as_ref()
                .cloned()
                .unwrap_or_default();
            // group_sections_by_page uses ResumeSection (with page_number on sub-entries)
            // order_sections returns Vec<&ResumeSection> — collect into owned Vec first.
            let ordered: Vec<ResumeSection> =
                crate::render::section_order::order_sections(&params.sections)
                    .into_iter()
                    .cloned()
                    .collect();
            let pages = group_sections_by_page(&ordered);
            let sections_latex = crate::templates::build_paginated_sections_latex(&pages, &fmt);
            crate::templates::render_file_template_with_sections(
                template,
                &profile,
                &sections,
                &sections_latex,
            )
        } else {
            crate::templates::render_file_template(template, &profile, &sections)
        }
    };

    if let Some(tid) = template_id {
        // Acquire read lock — lightweight, no contention in practice
        let cache = template_cache.read().await;
        if let Some(template) = cache.get(tid) {
            return render_with_template(template);
        } else {
            // template_id set but not in cache — log and fall through to generic-cv
            warn!(
                "Resume {} has template_id '{}' but it's not in the cache — falling back to generic-cv",
                params.resume_id, tid
            );
        }
    }

    // None branch: try generic-cv from cache first.
    {
        let cache = template_cache.read().await;
        if let Some(template) = cache.get("generic-cv") {
            return render_with_template(template);
        }
    }

    // Last-resort fallback: build a minimal pdflatex document without fontspec.
    // This path is taken when the template cache is empty (e.g. templates dir missing).
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
    use crate::render::escape::escape_latex;

    let mut doc = String::with_capacity(4096);

    doc.push_str(
        r#"\documentclass[letterpaper,11pt]{article}
\usepackage{lmodern}
\usepackage[T1]{fontenc}
\usepackage{textcomp}
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
        doc.push_str(&format!("\n\\section{{{}}}\n", escape_latex(&section.name)));
        for sub in &section.sub_entries {
            if let Some(h) = &sub.header_latex {
                doc.push_str(h);
                doc.push('\n');
            }
            if !sub.bullets.is_empty() {
                doc.push_str("\\begin{itemize}\n");
                for bullet in &sub.bullets {
                    doc.push_str(&format!("  \\item {}\n", escape_latex(bullet)));
                }
                doc.push_str("\\end{itemize}\n");
            }
            if sub.header_latex.is_some() {
                doc.push_str("\\vspace{2pt}\n");
            }
        }
    }

    doc.push_str("\n\\end{document}\n");
    doc
}

/// Fetches the user's profile data for a given resume.
///
/// Queries `user_profiles` via a JOIN on `resumes.user_id`.
/// `selected_link_types`: if Some, only include links whose `type` is in the list.
///   If None, include all links.
/// Returns `ProfileData::default()` if no profile row exists.
async fn fetch_user_profile(
    db: &PgPool,
    resume_id: Uuid,
    selected_link_types: Option<&[String]>,
) -> anyhow::Result<ProfileData> {
    use crate::models::user::{ProfileLink, UserProfile};

    let row = sqlx::query_as::<_, UserProfile>(
        r#"SELECT up.*
           FROM user_profiles up
           JOIN resumes r ON r.user_id = up.user_id
           WHERE r.id = $1"#,
    )
    .bind(resume_id)
    .fetch_optional(db)
    .await?;

    let Some(p) = row else {
        return Ok(ProfileData::default());
    };

    let all_links: Vec<ProfileLink> = serde_json::from_value(p.links).unwrap_or_default();

    let header_links: Vec<(String, String)> = all_links
        .into_iter()
        .filter(|link| {
            selected_link_types
                .map(|types| types.iter().any(|t| t == &link.link_type))
                .unwrap_or(true)
        })
        .map(|link| {
            let display = link
                .alias
                .filter(|a| !a.is_empty())
                .or_else(|| link.label.filter(|l| !l.is_empty()))
                .unwrap_or_else(|| link.link_type.clone());
            (display, link.url)
        })
        .collect();

    Ok(ProfileData {
        full_name: p.full_name,
        email: p.email,
        phone: p.phone,
        location: p.location,
        header_links,
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

/// Drops the last (lowest-scoring) sub-entry from the last non-empty section in `params`.
///
/// Called by the post-render overflow guard when pdflatex produces more pages than expected.
/// Removes at most one sub-entry per call; the caller loops up to `max_post_render_retries`.
///
/// Sections are visited in reverse order so that appended sections (skills, etc.) are
/// trimmed before core experience entries. Within a section, the last sub-entry (lowest
/// relevance-ranked) is removed. Empty sections are cleaned up after removal.
fn drop_last_sub_entry(params: &mut RenderParams) {
    // Walk sections in reverse order; find the first one with sub-entries.
    for section in params.sections.iter_mut().rev() {
        if !section.sub_entries.is_empty() {
            section.sub_entries.pop();
            return; // Only one sub-entry per call
        }
    }
    // If all sections are already empty, nothing to drop.
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
        // Empty template cache — no templates needed for this not-found test
        let empty_cache = Arc::new(TemplateCache::new(std::collections::HashMap::new()));
        let result = fetch_render_data(&pool, fake_id, &empty_cache).await;
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

    fn make_profile(name: &str) -> ProfileData {
        ProfileData {
            full_name: name.to_string(),
            email: "test@example.com".to_string(),
            phone: "555-0100".to_string(),
            location: "Remote".to_string(),
            header_links: vec![],
        }
    }

    fn make_sections(names_and_bullets: &[(&str, &[&str])]) -> Vec<ResumeSection> {
        names_and_bullets
            .iter()
            .map(|(name, bullets)| {
                ResumeSection::flat(
                    name.to_string(),
                    bullets.iter().map(|b| b.to_string()).collect(),
                )
            })
            .collect()
    }

    #[test]
    fn test_compute_render_hash_deterministic() {
        let profile = make_profile("Alice");
        let sections = make_sections(&[("Experience", &["Built thing A", "Shipped thing B"])]);
        let h1 = compute_render_hash(Some("generic-cv"), &profile, &sections);
        let h2 = compute_render_hash(Some("generic-cv"), &profile, &sections);
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_compute_render_hash_differs_on_template_change() {
        let profile = make_profile("Alice");
        let sections = make_sections(&[("Experience", &["Built thing A"])]);
        let h1 = compute_render_hash(Some("generic-cv"), &profile, &sections);
        let h2 = compute_render_hash(Some("hacker"), &profile, &sections);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_compute_render_hash_differs_on_bullet_change() {
        let profile = make_profile("Alice");
        let sections_a = make_sections(&[("Experience", &["Built thing A"])]);
        let sections_b = make_sections(&[("Experience", &["Built thing B"])]);
        let h1 = compute_render_hash(Some("generic-cv"), &profile, &sections_a);
        let h2 = compute_render_hash(Some("generic-cv"), &profile, &sections_b);
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_compute_render_hash_stable_across_section_order() {
        let profile = make_profile("Alice");
        // Same sections, different insertion order
        let sections_ab =
            make_sections(&[("Experience", &["Built A"]), ("Education", &["B.S. CS"])]);
        let sections_ba =
            make_sections(&[("Education", &["B.S. CS"]), ("Experience", &["Built A"])]);
        let h1 = compute_render_hash(Some("generic-cv"), &profile, &sections_ab);
        let h2 = compute_render_hash(Some("generic-cv"), &profile, &sections_ba);
        assert_eq!(
            h1, h2,
            "Hash must be stable regardless of section insertion order"
        );
    }
}
