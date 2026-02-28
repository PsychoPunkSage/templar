use anyhow::Result;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::completeness::compute_completeness_report;
use crate::context::dedup::{check_for_conflicts, ConflictWarning};
use crate::context::prompts::{CONTEXT_PARSE_PROMPT, CONTEXT_PARSE_SYSTEM};
use crate::context::scoring::compute_recency_score;
use crate::context::validation::{validate_impact, ImpactValidationResult};
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
    pub impact_validation: ImpactValidationResult,
    pub conflict_warnings: Vec<ConflictWarning>,
}

#[derive(Debug, Deserialize)]
pub struct IngestConfirmRequest {
    pub entry: serde_json::Value,
    pub user_id: Uuid,
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
}

pub async fn parse_and_validate(
    raw_text: &str,
    llm: &LlmClient,
    pool: &sqlx::PgPool,
    user_id: Uuid,
) -> Result<IngestPreviewResponse, AppError> {
    let prompt = CONTEXT_PARSE_PROMPT.replace("{raw_text}", raw_text);
    let parsed: serde_json::Value = llm
        .call_json(&prompt, CONTEXT_PARSE_SYSTEM)
        .await
        .map_err(|e| AppError::Llm(format!("Failed to parse context entry: {e}")))?;

    let bullets = extract_bullets(&parsed);
    let all_results: Vec<_> = bullets.iter().map(|b| validate_impact(b)).collect();
    let impact_validation = ImpactValidationResult {
        passed: all_results.iter().all(|r| r.passed),
        missing: all_results.iter().flat_map(|r| r.missing.clone()).collect(),
        suggestions: all_results
            .iter()
            .flat_map(|r| r.suggestions.clone())
            .collect(),
    };

    let existing = get_current_entries(pool, user_id)
        .await
        .map_err(AppError::Internal)?;
    let entry_type = parsed
        .get("entry_type")
        .and_then(|v| v.as_str())
        .unwrap_or("experience");
    let data = parsed.get("data").cloned().unwrap_or_default();
    let conflict_warnings = check_for_conflicts(&existing, entry_type, &data);

    Ok(IngestPreviewResponse {
        entry: parsed,
        impact_validation,
        conflict_warnings,
    })
}

pub async fn confirm_ingest(
    pool: &sqlx::PgPool,
    s3: &aws_sdk_s3::Client,
    s3_bucket: &str,
    request: &IngestConfirmRequest,
) -> Result<IngestConfirmResponse, AppError> {
    let user_id = request.user_id;
    let entry = &request.entry;

    let entry_type = entry
        .get("entry_type")
        .and_then(|v| v.as_str())
        .unwrap_or("experience")
        .to_string();
    let data = entry.get("data").cloned().unwrap_or_default();

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

    // Completeness before insert
    let entries_before = get_current_entries(pool, user_id)
        .await
        .map_err(AppError::Internal)?;
    let score_before = compute_completeness_report(&entries_before).overall_score;

    let version = commit_context_update(
        pool,
        s3,
        s3_bucket,
        CommitParams {
            user_id,
            entry_id,
            entry_type: &entry_type,
            data: &data,
            raw_text: None,
            recency_score,
            impact_score,
            tags: &tags,
            flagged_evergreen,
            contribution_type: &contribution_type,
        },
    )
    .await
    .map_err(AppError::Internal)?;

    // Completeness after insert
    let entries_after = get_current_entries(pool, user_id)
        .await
        .map_err(AppError::Internal)?;
    let score_after = compute_completeness_report(&entries_after).overall_score;

    Ok(IngestConfirmResponse {
        entry_id,
        version: version.version,
        completeness_delta: score_after - score_before,
    })
}

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
    let quantified = bullets.iter().filter(|b| validate_impact(b).passed).count();
    (quantified as f64 / bullets.len() as f64).clamp(0.0, 1.0)
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
