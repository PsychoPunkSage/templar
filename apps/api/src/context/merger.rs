//! Context entry merger — Phase 5.5.4.
//!
//! When a new context upload duplicates an existing entry, this module merges
//! them into a single, richer entry (new version of the existing entry_id).
//!
//! The LLM merge prompt asks Claude to combine both descriptions, preserving all
//! unique details from each while eliminating redundancy.

use anyhow::Result;
use aws_sdk_s3::Client as S3Client;
use sqlx::PgPool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::context::prompts::{MERGE_ENTRIES_PROMPT, MERGE_ENTRIES_SYSTEM};
use crate::context::scoring::compute_recency_score;
use crate::context::validation::validate_bullets;
use crate::context::versioning::{commit_context_update, get_current_entries, CommitParams};
use crate::llm_client::LlmClient;

/// Merge `new_entry` into the existing entry identified by `existing_entry_id`.
///
/// Steps:
/// 1. Fetch the existing entry's data from DB
/// 2. Call LLM to produce a merged entry JSON
/// 3. Commit the merged entry as a new version (append-only)
///
/// The existing `entry_id` is preserved — only the version increments.
pub async fn merge_and_commit(
    db: &PgPool,
    s3: &S3Client,
    s3_bucket: &str,
    llm: &LlmClient,
    user_id: Uuid,
    existing_entry_id: Uuid,
    new_entry: &serde_json::Value,
) -> Result<()> {
    // Step 1: Fetch existing entry from DB
    let existing_entries = get_current_entries(db, user_id).await?;
    let existing = existing_entries
        .iter()
        .find(|e| e.entry_id == existing_entry_id)
        .ok_or_else(|| anyhow::anyhow!("Existing entry {existing_entry_id} not found for merge"))?;

    let existing_json = serde_json::json!({
        "entry_type": existing.entry_type,
        "data": existing.data,
    });

    // Step 2: LLM merge
    let prompt = MERGE_ENTRIES_PROMPT
        .replace(
            "{existing_json}",
            &serde_json::to_string_pretty(&existing_json).unwrap_or_default(),
        )
        .replace(
            "{new_json}",
            &serde_json::to_string_pretty(new_entry).unwrap_or_default(),
        );

    let merged: serde_json::Value = match llm.call_json(&prompt, MERGE_ENTRIES_SYSTEM).await {
        Ok(v) => v,
        Err(e) => {
            warn!(%existing_entry_id, error = %e, "Merger: LLM merge failed, using new_entry as-is");
            new_entry.clone()
        }
    };

    // Step 3: Commit merged entry as a new version of the existing entry_id
    let entry_type = merged
        .get("entry_type")
        .and_then(|v| v.as_str())
        .unwrap_or(&existing.entry_type)
        .to_string();
    let data = merged.get("data").cloned().unwrap_or(existing.data.clone());

    let contribution_type = data
        .get("contribution_type")
        .and_then(|v| v.as_str())
        .unwrap_or(&existing.contribution_type)
        .to_string();

    let end_date = data
        .get("date_end")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
    let flagged_evergreen = matches!(entry_type.as_str(), "skill" | "certification");
    let recency_score = compute_recency_score(end_date, flagged_evergreen, 18.0);

    let bullets = extract_bullets_from_data(&data);
    let impact_score = compute_impact_score(&bullets);
    let quality = validate_bullets(&bullets);
    let quality_flags = quality.flags.clone();
    let tags = extract_tags(&data, &entry_type);

    commit_context_update(
        db,
        s3,
        s3_bucket,
        CommitParams {
            user_id,
            entry_id: existing_entry_id, // preserve the stable entry_id
            entry_type: &entry_type,
            data: &data,
            raw_text: None,
            recency_score,
            impact_score,
            tags: &tags,
            flagged_evergreen,
            contribution_type: &contribution_type,
            quality_score: quality.quality_score as f64,
            quality_flags: &quality_flags,
        },
    )
    .await?;

    info!(%existing_entry_id, "Merger: new version committed");
    Ok(())
}

fn compute_impact_score(bullets: &[String]) -> f64 {
    use crate::context::validation::validate_impact;
    if bullets.is_empty() {
        return 0.5;
    }
    let total: f32 = bullets
        .iter()
        .map(|b| validate_impact(b).quality_score)
        .sum();
    (total as f64 / bullets.len() as f64).clamp(0.0, 1.0)
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
