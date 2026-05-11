use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::context::completeness::compute_completeness_report;
use crate::context::dedup::{check_for_conflicts, ConflictWarning};
use crate::context::prompts::{
    CONTEXT_BULLET_PROMPT, CONTEXT_BULLET_SYSTEM, CONTEXT_META_PROMPT, CONTEXT_META_SYSTEM,
    CONTEXT_PARSE_PROMPT, CONTEXT_PARSE_SYSTEM,
};
use crate::context::scoring::compute_recency_score;
use crate::context::validation::{validate_bullets, validate_impact, validate_required_fields, ImpactQuality};
use crate::context::versioning::{commit_context_update, get_current_entries, CommitParams};
use crate::errors::AppError;
use crate::llm_client::LlmClient;

#[derive(Debug, Deserialize)]
pub struct IngestRequest {
    pub raw_text: String,
    pub user_id: Uuid,
}

#[derive(Debug, Serialize)]
pub struct IngestPreviewResponse {
    pub entry: serde_json::Value,
    /// Phase 5.5: non-blocking quality assessment (replaces pass/fail validation).
    pub quality: ImpactQuality,
    pub conflict_warnings: Vec<ConflictWarning>,
}

#[derive(Debug, Deserialize)]
pub struct IngestConfirmRequest {
    pub entry: serde_json::Value,
    pub user_id: Uuid,
    /// Raw source text stored in DB so per-entry LLM generation has a source.
    #[serde(default)]
    pub raw_text: Option<String>,
    // Acknowledged gaps are accepted from the client but not yet processed server-side.
    // They are preserved for future audit logging. See Phase 5 grounding system.
    #[allow(dead_code)]
    pub acknowledged_gaps: Vec<AcknowledgedGap>,
}

#[derive(Debug, Deserialize)]
pub struct AcknowledgedGap {
    #[allow(dead_code)]
    pub bullet: String,
    #[allow(dead_code)]
    pub acknowledgement: String,
}

#[derive(Debug, Serialize)]
pub struct IngestConfirmResponse {
    pub entry_id: Uuid,
    pub version: i32,
    pub completeness_delta: f64,
    /// Phase 5.5: quality hints to display in the UI.
    pub improvement_hints: Vec<String>,
}

#[tracing::instrument(skip(llm, pool), fields(user_id = %user_id, text_len = raw_text.len()))]
pub async fn parse_and_validate(
    raw_text: &str,
    llm: &LlmClient,
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<IngestPreviewResponse, AppError> {
    tracing::info!("starting context parse and validate");

    tracing::debug!("calling LLM for context parse");
    let prompt = CONTEXT_PARSE_PROMPT.replace("{raw_text}", raw_text);
    let parsed: serde_json::Value = llm
        .call_json(&prompt, CONTEXT_PARSE_SYSTEM)
        .await
        .map_err(|e| AppError::Llm(format!("Failed to parse context entry: {e}")))?;
    tracing::debug!("LLM parse complete, computing quality");

    // Phase 5.5: quality assessment is non-blocking — we always proceed
    let bullets = extract_bullets(&parsed);
    let quality = if bullets.is_empty() {
        validate_impact(raw_text)
    } else {
        let per_bullet: Vec<_> = bullets.iter().map(|b| validate_impact(b)).collect();
        ImpactQuality::aggregate(&per_bullet)
    };

    tracing::debug!(
        quality_score = quality.quality_score,
        flags = ?quality.flags,
        "quality assessment complete, checking for conflicts"
    );

    let existing = get_current_entries(pool, user_id)
        .await
        .map_err(AppError::Internal)?;
    let entry_type = parsed
        .get("entry_type")
        .and_then(|v| v.as_str())
        .unwrap_or("experience");
    let data = parsed.get("data").cloned().unwrap_or_default();
    let field_quality = validate_required_fields(entry_type, &data);
    let quality = ImpactQuality::aggregate(&[quality, field_quality]);
    let conflict_warnings = check_for_conflicts(&existing, entry_type, &data);

    tracing::info!(
        quality_score = quality.quality_score,
        conflict_count = conflict_warnings.len(),
        "parse_and_validate complete"
    );

    Ok(IngestPreviewResponse {
        entry: parsed,
        quality,
        conflict_warnings,
    })
}

#[tracing::instrument(skip(pool, s3), fields(user_id = %request.user_id))]
pub async fn confirm_ingest(
    pool: &sqlx::PgPool,
    s3: &aws_sdk_s3::Client,
    s3_bucket: &str,
    request: &IngestConfirmRequest,
) -> Result<IngestConfirmResponse, AppError> {
    tracing::info!("starting context commit to DB and S3");
    let user_id = request.user_id;
    let entry = &request.entry;

    let entry_type = entry
        .get("entry_type")
        .and_then(|v| v.as_str())
        .unwrap_or("experience")
        .to_string();
    let data = entry.get("data").cloned().unwrap_or_default();

    // Guard: never store null data — can happen if LLM parse failed silently
    if data.is_null() {
        return Err(AppError::Validation(
            "Failed to extract structured data from this entry. \
             The content may be too short or improperly formatted."
                .into(),
        ));
    }

    let entry_id = Uuid::new_v4();
    let contribution_type = data
        .get("contribution_type")
        .and_then(|v| v.as_str())
        .unwrap_or("team_member")
        .to_string();

    let end_date = data
        .get("date_end")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    let flagged_evergreen = matches!(entry_type.as_str(), "skill" | "certification");
    let recency_score = compute_recency_score(end_date, flagged_evergreen, 18.0);

    let bullets = extract_bullets_from_data(&data);
    let impact_score = compute_impact_score(&bullets);
    let tags = extract_tags(&data, &entry_type);

    // Phase 5.5: compute quality for storage
    let bullet_quality = validate_bullets(&bullets);
    let field_quality = validate_required_fields(&entry_type, &data);
    let quality = ImpactQuality::aggregate(&[bullet_quality, field_quality]);
    let quality_flags = quality.flags.clone();

    // Completeness before insert
    let entries_before = get_current_entries(pool, user_id)
        .await
        .map_err(AppError::Internal)?;
    let score_before = compute_completeness_report(&entries_before).overall_score;

    tracing::debug!(%entry_id, %entry_type, "committing context entry to DB and S3");
    let version = commit_context_update(
        pool,
        s3,
        s3_bucket,
        CommitParams {
            user_id,
            entry_id,
            entry_type: &entry_type,
            data: &data,
            raw_text: request.raw_text.as_deref(),
            recency_score,
            impact_score,
            tags: &tags,
            flagged_evergreen,
            contribution_type: &contribution_type,
            quality_score: quality.quality_score as f64,
            quality_flags: &quality_flags,
        },
    )
    .await
    .map_err(AppError::Internal)?;

    // Completeness after insert
    let entries_after = get_current_entries(pool, user_id)
        .await
        .map_err(AppError::Internal)?;
    let score_after = compute_completeness_report(&entries_after).overall_score;

    let completeness_delta = score_after - score_before;
    tracing::info!(
        %entry_id,
        version = version.version,
        completeness_delta,
        quality_score = quality.quality_score,
        "context entry committed successfully"
    );

    Ok(IngestConfirmResponse {
        entry_id,
        version: version.version,
        completeness_delta,
        improvement_hints: quality.suggestions,
    })
}

// ────────────────────────────────────────────────────────────────────────────
// Three-phase ingestion pipeline
// ────────────────────────────────────────────────────────────────────────────

/// Splits text into chunks that fit within `budget` tokens (est. len/4).
///
/// Strategy (in order):
/// 1. If whole text fits → return single chunk
/// 2. Split on `\n\n` (paragraph boundaries), group greedily under budget
/// 3. Add 3-line overlap between consecutive chunks
/// 4. Fallback: 40-line chunks with 3-line overlap if paragraphs don't help
pub(crate) fn chunk_text_for_bullets(text: &str, budget: usize) -> Vec<String> {
    let estimated_tokens = text.len() / 4;

    // Case 1: fits as-is
    if estimated_tokens <= budget {
        return vec![text.to_string()];
    }

    // Case 2: paragraph-boundary splitting with overlap
    let paragraphs: Vec<&str> = text.split("\n\n").collect();
    if paragraphs.len() > 1 {
        let mut chunks: Vec<String> = vec![];
        let mut current = String::new();
        let mut overlap_lines: Vec<String> = vec![];

        for para in &paragraphs {
            let candidate = if current.is_empty() {
                para.to_string()
            } else {
                format!("{current}\n\n{para}")
            };

            if candidate.len() / 4 <= budget {
                current = candidate;
            } else {
                if !current.is_empty() {
                    // Collect last 3 lines as owned Strings before moving current
                    let mut tail: Vec<String> =
                        current.lines().rev().take(3).map(String::from).collect();
                    tail.reverse();
                    overlap_lines = tail;
                    chunks.push(current.clone());
                }
                // Start next chunk with overlap
                let overlap = overlap_lines.join("\n");
                current = if overlap.is_empty() {
                    para.to_string()
                } else {
                    format!("{overlap}\n\n{para}")
                };
            }
        }
        if !current.is_empty() {
            chunks.push(current);
        }
        if !chunks.is_empty() {
            return chunks;
        }
    }

    // Case 3/4: line-based fallback — 40-line chunks with 3-line overlap
    let lines: Vec<&str> = text.lines().collect();
    let chunk_size = 40;
    let overlap = 3;
    let mut chunks: Vec<String> = vec![];
    let mut start = 0;
    while start < lines.len() {
        let end = (start + chunk_size).min(lines.len());
        chunks.push(lines[start..end].join("\n"));
        if end >= lines.len() {
            break;
        }
        start = end.saturating_sub(overlap);
    }
    chunks
}

/// Three-phase LLM parse for a single context entry text.
///
/// Phase A: metadata extraction (1 small call — never hits token ceiling)
/// Phase B: bullet extraction (chunked — failures are non-fatal)
/// Phase C: assemble into `{entry_type, data}` structure (pure Rust)
///
/// `sem` is a shared semaphore across all ingest workers — each individual LLM call
/// acquires one permit and releases it immediately after the call completes. This caps
/// total concurrent LLM calls (across all workers) to `sem.available_permits()` at any
/// instant, preventing 429 rate-limit errors.
///
/// `bullet_token_budget` controls the chunk size for Phase B calls (replaces the
/// previously hardcoded 1500). Set via `Config::bullet_token_budget`.
///
/// Returns `Ok(assembled_entry)` or `Err` if Phase A failed.
pub(crate) async fn parse_three_phase(
    raw_text: &str,
    llm: &LlmClient,
    sem: &Arc<Semaphore>,
    bullet_token_budget: usize,
) -> Result<serde_json::Value, AppError> {
    // ── Phase A: metadata ────────────────────────────────────────────────────
    // Acquire permit → call LLM → release permit immediately (before Phase B).
    let _permit = sem.acquire().await.expect("ingest semaphore closed");
    let meta_prompt = CONTEXT_META_PROMPT.replace("{raw_text}", raw_text);
    let meta: serde_json::Value = llm
        .call_json(&meta_prompt, CONTEXT_META_SYSTEM)
        .await
        .map_err(|e| AppError::Llm(format!("Phase A metadata extraction failed: {e}")))?;
    drop(_permit); // Release before Phase B calls

    let entry_type = meta
        .get("entry_type")
        .and_then(|v| v.as_str())
        .unwrap_or("experience")
        .to_string();

    // ── Phase B: bullet extraction (chunked, non-fatal) ───────────────────────
    // Each chunk acquires and immediately releases the permit.
    let chunks = chunk_text_for_bullets(raw_text, bullet_token_budget);
    let mut all_bullets: Vec<serde_json::Value> = vec![];

    for (i, chunk) in chunks.iter().enumerate() {
        let _permit = sem.acquire().await.expect("ingest semaphore closed");
        let prompt = CONTEXT_BULLET_PROMPT.replace("{raw_text}", chunk);
        match llm
            .call_json::<serde_json::Value>(&prompt, CONTEXT_BULLET_SYSTEM)
            .await
        {
            Ok(v) => {
                if let Some(bullets) = v.get("bullets").and_then(|b| b.as_array()) {
                    all_bullets.extend(bullets.clone());
                }
            }
            Err(e) => {
                tracing::warn!(chunk = i, error = %e, "Phase B bullet chunk failed (non-fatal), skipping");
            }
        }
        drop(_permit); // Release before next chunk
    }

    // Deduplicate by exact text match
    let mut seen_texts: std::collections::HashSet<String> = std::collections::HashSet::new();
    all_bullets.retain(|b| {
        let text = b
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        seen_texts.insert(text)
    });

    // ── Phase C: assemble (pure Rust) ────────────────────────────────────────
    let mut data = meta.as_object().cloned().unwrap_or_default();

    // Remove entry_type from data (it belongs at the top level)
    data.remove("entry_type");

    // Drop null values to keep data clean
    data.retain(|_, v| !v.is_null());

    // Attach bullets to bullet-bearing entry types
    let bullet_types = ["experience", "project", "open_source", "extracurricular"];
    if bullet_types.contains(&entry_type.as_str()) && !all_bullets.is_empty() {
        data.insert("bullets".into(), serde_json::Value::Array(all_bullets));
    }

    Ok(serde_json::json!({ "entry_type": entry_type, "data": data }))
}

// ────────────────────────────────────────────────────────────────────────────
// Private helpers
// ────────────────────────────────────────────────────────────────────────────

fn extract_bullets(entry: &serde_json::Value) -> Vec<String> {
    entry
        .get("data")
        .map(extract_bullets_from_data)
        .unwrap_or_default()
}

fn extract_bullets_from_data(data: &serde_json::Value) -> Vec<String> {
    data.get("bullets")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn compute_impact_score(bullets: &[String]) -> f64 {
    if bullets.is_empty() {
        return 0.5;
    }
    let total_quality: f32 = bullets
        .iter()
        .map(|b| validate_impact(b).quality_score)
        .sum();
    (total_quality as f64 / bullets.len() as f64).clamp(0.0, 1.0)
}

fn extract_tags(data: &serde_json::Value, entry_type: &str) -> Vec<String> {
    let mut tags = vec![entry_type.to_string()];
    for field in ["tech_stack", "items"] {
        if let Some(arr) = data.get(field).and_then(|v| v.as_array()) {
            for item in arr {
                if let Some(s) = item.as_str() {
                    tags.push(s.to_lowercase());
                }
            }
        }
    }
    for field in ["company", "institution", "project_name", "organization"] {
        if let Some(s) = data.get(field).and_then(|v| v.as_str()) {
            tags.push(s.to_lowercase());
        }
    }
    tags.sort();
    tags.dedup();
    tags
}
