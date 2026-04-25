//! Interview prep generation pipeline.
//!
//! Main entry point: `generate_prep(db, llm, project_id)`.
//!
//! Pipeline:
//!   1. Fetch project → current_resume_id (bail if null)
//!   2. Fetch resume bullets from resume_bullets WHERE resume_id = current_resume_id
//!   3. Fetch existing interview_prep_bullets for project (for diff)
//!   4. Flip interview_prep_meta.status = generating (already done by enqueue_prep_job, but re-confirm)
//!   5. Diff new vs old bullet hashes
//!   6. Reuse scaffolds for unchanged bullets
//!   7. LLM call for new/modified bullets (STAR + questions)
//!   8. LLM call for gap questions (from FitReport in generation_jobs.result) + company extraction
//!   9. Upsert interview_prep_bullets; delete removed hashes
//!  10. Upsert interview_prep_meta status = ready, expires_at = NOW()+30d

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use tracing::{info, warn};
use uuid::Uuid;

use crate::interview_prep::models::{
    BulletPrepLlmOutput, CompanyContext, GapQuestion, GapQuestionsLlmOutput, PrepBulletRow,
    StarScaffold,
};
use crate::interview_prep::prompts::{
    GAP_QUESTIONS_SYSTEM, GAP_QUESTIONS_TEMPLATE, STAR_SCAFFOLD_SYSTEM, STAR_SCAFFOLD_TEMPLATE,
};
use crate::llm_client::LlmClient;

// ────────────────────────────────────────────────────────────────────────────
// Public helpers — exposed for unit tests
// ────────────────────────────────────────────────────────────────────────────

/// Compute the bullet hash: SHA-256 of trimmed lowercase bullet text.
///
/// This is the stable identity key for a bullet across resume regenerations.
/// Two bullets with identical normalised text share a hash and reuse their scaffold.
pub fn hash_bullet(text: &str) -> String {
    let normalised = text.trim().to_lowercase();
    let mut hasher = Sha256::new();
    hasher.update(normalised.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Result of diffing old vs new bullet sets.
#[derive(Debug, Default)]
pub struct BulletDiff {
    /// Hashes present in both old and new — scaffold can be reused.
    pub unchanged: HashSet<String>,
    /// Hashes present in new but not old — need new LLM call.
    pub added: Vec<NewBullet>,
    /// Hashes in old but not new — rows to DELETE.
    pub removed: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct NewBullet {
    pub hash: String,
    pub text: String,
    pub source_entry_id: Option<Uuid>,
}

/// Diff old prep bullet rows against new resume bullet texts.
///
/// Arguments:
///   old — existing PrepBulletRow slice from interview_prep_bullets
///   new — slice of (bullet_text, source_entry_id) pairs from resume_bullets
pub fn diff_bullets(old: &[PrepBulletRow], new: &[(String, Option<Uuid>)]) -> BulletDiff {
    let old_map: HashMap<String, &PrepBulletRow> =
        old.iter().map(|r| (r.bullet_hash.clone(), r)).collect();

    let new_hashes: HashSet<String> = new
        .iter()
        .map(|(text, _)| hash_bullet(text))
        .collect();

    let unchanged: HashSet<String> = old_map
        .keys()
        .filter(|h| new_hashes.contains(*h))
        .cloned()
        .collect();

    let removed: Vec<String> = old_map
        .keys()
        .filter(|h| !new_hashes.contains(*h))
        .cloned()
        .collect();

    let added: Vec<NewBullet> = new
        .iter()
        .filter_map(|(text, entry_id)| {
            let h = hash_bullet(text);
            if old_map.contains_key(&h) {
                None // unchanged
            } else {
                Some(NewBullet {
                    hash: h,
                    text: text.clone(),
                    source_entry_id: *entry_id,
                })
            }
        })
        .collect();

    BulletDiff { unchanged, added, removed }
}

// ────────────────────────────────────────────────────────────────────────────
// Main generation pipeline
// ────────────────────────────────────────────────────────────────────────────

/// Run the full interview prep generation pipeline for a project.
///
/// This is called from the background worker — it runs to completion or returns
/// an error. The caller (job.rs) is responsible for updating status to failed.
pub async fn generate_prep(db: &PgPool, llm: &LlmClient, project_id: Uuid) -> Result<()> {
    info!(project_id = %project_id, "Interview prep generation starting");

    // Step 1: Fetch project → current_resume_id
    let current_resume_id: Option<Uuid> = sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT current_resume_id FROM cv_projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(db)
    .await?
    .flatten();

    let resume_id = match current_resume_id {
        Some(id) => id,
        None => anyhow::bail!("Project {project_id} has no current_resume_id — cannot prep"),
    };

    // Step 2: Fetch resume bullets
    let bullet_rows: Vec<(String, Option<Uuid>)> = sqlx::query_as::<_, (String, Option<Uuid>)>(
        "SELECT bullet_text, source_entry_id FROM resume_bullets WHERE resume_id = $1 ORDER BY order_idx ASC NULLS LAST",
    )
    .bind(resume_id)
    .fetch_all(db)
    .await?;

    if bullet_rows.is_empty() {
        anyhow::bail!("Resume {resume_id} has no bullets — cannot prep");
    }

    // Step 3: Fetch existing prep bullets for diff
    let existing_rows: Vec<PrepBulletRow> = sqlx::query_as::<_, PrepBulletRow>(
        "SELECT id, project_id, bullet_hash, bullet_text, context_entry_id, star_scaffold, questions, created_at, updated_at
         FROM interview_prep_bullets
         WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_all(db)
    .await?;

    // Step 4: Diff old vs new
    let diff = diff_bullets(&existing_rows, &bullet_rows);

    info!(
        project_id = %project_id,
        unchanged = diff.unchanged.len(),
        added = diff.added.len(),
        removed = diff.removed.len(),
        "Bullet diff computed"
    );

    // Step 5: Delete removed bullet hashes
    if !diff.removed.is_empty() {
        sqlx::query(
            "DELETE FROM interview_prep_bullets WHERE project_id = $1 AND bullet_hash = ANY($2)",
        )
        .bind(project_id)
        .bind(&diff.removed)
        .execute(db)
        .await?;
    }

    // Step 6: Fetch JD text + generation job result (for FitReport)
    let jd_text: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT last_jd_text FROM cv_projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(db)
    .await?
    .flatten();

    let fit_report_json: Option<serde_json::Value> = fetch_fit_report_for_project(db, project_id)
        .await
        .unwrap_or(None);

    // Step 7: Load context entry lookup (entry_id → contribution_type + raw_text)
    let context_map = load_context_map(db, project_id).await.unwrap_or_default();

    // Step 8: LLM calls for added bullets (STAR + questions)
    for bullet in &diff.added {
        let (contribution_type, context_entry_text) = context_map
            .get(&bullet.source_entry_id.unwrap_or(Uuid::nil()))
            .map(|(ct, raw)| (ct.as_str(), raw.as_str()))
            .unwrap_or(("team_member", "No source context available."));

        let prompt = build_star_prompt(
            &bullet.text,
            context_entry_text,
            contribution_type,
            &jd_keywords_from_fit_report(&fit_report_json),
        );

        let llm_output: BulletPrepLlmOutput = match llm.call_json(&prompt, STAR_SCAFFOLD_SYSTEM).await {
            Ok(v) => v,
            Err(e) => {
                warn!(
                    project_id = %project_id,
                    bullet_hash = %bullet.hash,
                    error = %e,
                    "STAR scaffold LLM call failed — using empty scaffold"
                );
                BulletPrepLlmOutput {
                    star_scaffold: StarScaffold {
                        situation: "Details to confirm.".into(),
                        task: "Details to confirm.".into(),
                        action: bullet.text.clone(),
                        result: "Details to confirm.".into(),
                        talking_points: vec![],
                    },
                    questions: vec![],
                }
            }
        };

        let scaffold_json = serde_json::to_value(&llm_output.star_scaffold)?;
        let questions_json = serde_json::to_value(&llm_output.questions)?;

        // Upsert into interview_prep_bullets
        sqlx::query(
            r#"INSERT INTO interview_prep_bullets
                   (project_id, bullet_hash, bullet_text, context_entry_id, star_scaffold, questions)
               VALUES ($1, $2, $3, $4, $5, $6)
               ON CONFLICT (project_id, bullet_hash) DO UPDATE SET
                   bullet_text      = EXCLUDED.bullet_text,
                   context_entry_id = EXCLUDED.context_entry_id,
                   star_scaffold    = EXCLUDED.star_scaffold,
                   questions        = EXCLUDED.questions,
                   updated_at       = NOW()"#,
        )
        .bind(project_id)
        .bind(&bullet.hash)
        .bind(&bullet.text)
        .bind(bullet.source_entry_id)
        .bind(&scaffold_json)
        .bind(&questions_json)
        .execute(db)
        .await?;
    }

    // Step 9: Gap questions + company extraction (single LLM call)
    let (gap_questions, company_context) = match &jd_text {
        Some(jd) => {
            generate_gap_questions_and_context(llm, &fit_report_json, jd).await
        }
        None => {
            warn!(project_id = %project_id, "No JD text — skipping gap questions");
            (vec![], None)
        }
    };

    let gap_json = serde_json::to_value(&gap_questions)?;
    let company_json = company_context
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?;

    // Step 10: Compute is_stale = false (just generated)
    // Stale detection: diff between current bullet hashes and stored hashes is now empty
    let new_hashes: Vec<String> = bullet_rows.iter().map(|(t, _)| hash_bullet(t)).collect();
    let is_stale = compute_is_stale(db, project_id, &new_hashes).await;

    // Step 11: Upsert meta as ready
    sqlx::query(
        r#"INSERT INTO interview_prep_meta
               (project_id, gap_questions, company_context, status, last_generated_at, expires_at, is_stale)
           VALUES ($1, $2, $3, 'ready', NOW(), NOW() + INTERVAL '30 days', $4)
           ON CONFLICT (project_id) DO UPDATE SET
               gap_questions     = EXCLUDED.gap_questions,
               company_context   = EXCLUDED.company_context,
               status            = 'ready',
               last_generated_at = NOW(),
               expires_at        = NOW() + INTERVAL '30 days',
               is_stale          = $4,
               updated_at        = NOW()"#,
    )
    .bind(project_id)
    .bind(&gap_json)
    .bind(&company_json)
    .bind(is_stale)
    .execute(db)
    .await?;

    info!(project_id = %project_id, "Interview prep generation complete");
    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────────

fn build_star_prompt(
    bullet_text: &str,
    context_entry: &str,
    contribution_type: &str,
    jd_keywords: &str,
) -> String {
    STAR_SCAFFOLD_TEMPLATE
        .replace("{bullet_text}", bullet_text)
        .replace("{context_entry}", context_entry)
        .replace("{contribution_type}", contribution_type)
        .replace("{jd_keywords}", jd_keywords)
}

fn jd_keywords_from_fit_report(fit_report_json: &Option<serde_json::Value>) -> String {
    let Some(v) = fit_report_json else {
        return "(none available)".to_string();
    };

    // Try to extract strong_matches[].dimension as keyword proxies
    let mut keywords: Vec<String> = vec![];
    if let Some(strong) = v.get("strong_matches").and_then(|v| v.as_array()) {
        for m in strong.iter().take(4) {
            if let Some(dim) = m.get("dimension").and_then(|d| d.as_str()) {
                keywords.push(dim.to_string());
            }
        }
    }
    if let Some(partial) = v.get("partial_matches").and_then(|v| v.as_array()) {
        for m in partial.iter().take(4) {
            if let Some(dim) = m.get("dimension").and_then(|d| d.as_str()) {
                keywords.push(dim.to_string());
            }
        }
    }

    if keywords.is_empty() {
        "(none available)".to_string()
    } else {
        keywords.join(", ")
    }
}

/// Fetch the FitReport JSON from the most recent generation_jobs.result for this project.
async fn fetch_fit_report_for_project(
    db: &PgPool,
    project_id: Uuid,
) -> Result<Option<serde_json::Value>> {
    // cv_projects -> generation_job_id -> generation_jobs.result -> .fit_report
    let result_value: Option<serde_json::Value> = sqlx::query_scalar::<_, serde_json::Value>(
        r#"SELECT gj.result
           FROM cv_projects cp
           JOIN generation_jobs gj ON gj.id = cp.generation_job_id
           WHERE cp.id = $1
             AND gj.result IS NOT NULL
           LIMIT 1"#,
    )
    .bind(project_id)
    .fetch_optional(db)
    .await?;

    Ok(result_value.and_then(|v| {
        v.get("fit_report").cloned()
    }))
}

/// Load a map of entry_id → (contribution_type_str, raw_text) for entries
/// associated with the project's context entries.
async fn load_context_map(
    db: &PgPool,
    project_id: Uuid,
) -> Result<HashMap<Uuid, (String, String)>> {
    // Get user_id from the project
    let user_id: Option<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "SELECT user_id FROM cv_projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(db)
    .await?;

    let Some(uid) = user_id else {
        return Ok(HashMap::new());
    };

    // Fetch latest version of each context entry
    let rows: Vec<(Uuid, String, Option<String>)> = sqlx::query_as::<_, (Uuid, String, Option<String>)>(
        r#"SELECT DISTINCT ON (entry_id) entry_id, contribution_type, raw_text
           FROM context_entries
           WHERE user_id = $1
           ORDER BY entry_id, created_at DESC"#,
    )
    .bind(uid)
    .fetch_all(db)
    .await?;

    let map = rows
        .into_iter()
        .map(|(id, ct, raw)| (id, (ct, raw.unwrap_or_default())))
        .collect();

    Ok(map)
}

async fn generate_gap_questions_and_context(
    llm: &LlmClient,
    fit_report_json: &Option<serde_json::Value>,
    jd_text: &str,
) -> (Vec<GapQuestion>, Option<CompanyContext>) {
    // Build gaps list
    let gaps_json = match fit_report_json {
        Some(v) => {
            let gaps = v.get("gaps").cloned().unwrap_or(serde_json::Value::Array(vec![]));
            // Remap to {area, description} shape for the prompt
            let remapped: Vec<serde_json::Value> = gaps
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .map(|g| {
                            serde_json::json!({
                                "area": g.get("keyword").and_then(|k| k.as_str()).unwrap_or("unknown"),
                                "description": g.get("suggestion").and_then(|s| s.as_str()).unwrap_or("No candidate evidence for this requirement.")
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            serde_json::to_string_pretty(&remapped).unwrap_or_else(|_| "[]".to_string())
        }
        None => "[]".to_string(),
    };

    // Truncate JD to ~2000 chars to keep prompt lean
    let jd_excerpt: String = jd_text.chars().take(2000).collect();

    let prompt = GAP_QUESTIONS_TEMPLATE
        .replace("{gaps_json}", &gaps_json)
        .replace("{jd_text}", &jd_excerpt);

    let output: GapQuestionsLlmOutput = match llm.call_json(&prompt, GAP_QUESTIONS_SYSTEM).await {
        Ok(v) => v,
        Err(e) => {
            warn!(error = %e, "Gap questions LLM call failed — returning empty");
            return (vec![], None);
        }
    };

    let company_context = {
        let name = output.company_name.as_deref().unwrap_or("").trim().to_string();
        if name.is_empty() {
            None
        } else {
            Some(CompanyContext {
                company_name: name,
                company_stage: output.company_stage.filter(|s| !s.trim().is_empty()),
                role_title: output.role_title.filter(|s| !s.trim().is_empty()),
            })
        }
    };

    (output.gap_questions, company_context)
}

/// Compute is_stale by comparing new bullet hashes against current stored hashes.
///
/// Returns true if there are differences (added or removed bullets since last prep).
async fn compute_is_stale(db: &PgPool, project_id: Uuid, new_hashes: &[String]) -> bool {
    let stored: Vec<String> = sqlx::query_scalar::<_, String>(
        "SELECT bullet_hash FROM interview_prep_bullets WHERE project_id = $1",
    )
    .bind(project_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();

    let stored_set: HashSet<&str> = stored.iter().map(|s| s.as_str()).collect();
    let new_set: HashSet<&str> = new_hashes.iter().map(|s| s.as_str()).collect();

    stored_set != new_set
}

// ────────────────────────────────────────────────────────────────────────────
// Questions-only refinement (called by PUT /:project_id/company)
// ────────────────────────────────────────────────────────────────────────────

/// Re-run only the gap questions LLM call after company context is updated.
/// STAR scaffolds are NOT regenerated (that would waste tokens).
pub async fn regenerate_gap_questions(
    db: &PgPool,
    llm: &LlmClient,
    project_id: Uuid,
) -> Result<()> {
    let jd_text: Option<String> = sqlx::query_scalar::<_, Option<String>>(
        "SELECT last_jd_text FROM cv_projects WHERE id = $1",
    )
    .bind(project_id)
    .fetch_optional(db)
    .await?
    .flatten();

    let fit_report_json = fetch_fit_report_for_project(db, project_id).await?;

    let (gap_questions, company_context) = match &jd_text {
        Some(jd) => generate_gap_questions_and_context(llm, &fit_report_json, jd).await,
        None => (vec![], None),
    };

    let gap_json = serde_json::to_value(&gap_questions)?;
    let company_json = company_context
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?;

    sqlx::query(
        r#"UPDATE interview_prep_meta
           SET gap_questions = $1,
               company_context = COALESCE($2, company_context),
               updated_at = NOW()
           WHERE project_id = $3"#,
    )
    .bind(&gap_json)
    .bind(&company_json)
    .bind(project_id)
    .execute(db)
    .await?;

    Ok(())
}

// ────────────────────────────────────────────────────────────────────────────
// Unit tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_row(hash: &str, text: &str) -> PrepBulletRow {
        PrepBulletRow {
            id: Uuid::new_v4(),
            project_id: Uuid::new_v4(),
            bullet_hash: hash.to_string(),
            bullet_text: text.to_string(),
            context_entry_id: None,
            star_scaffold: serde_json::Value::Object(serde_json::Map::new()),
            questions: serde_json::Value::Array(vec![]),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn test_hash_bullet_normalizes() {
        let h1 = hash_bullet("  Built a distributed system  ");
        let h2 = hash_bullet("built a distributed system");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_bullet_different_texts() {
        let h1 = hash_bullet("Led backend services");
        let h2 = hash_bullet("led backend services but different");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_diff_empty_old_all_added() {
        let old: Vec<PrepBulletRow> = vec![];
        let new = vec![
            ("Built API".to_string(), None),
            ("Led team".to_string(), None),
        ];
        let diff = diff_bullets(&old, &new);
        assert_eq!(diff.unchanged.len(), 0);
        assert_eq!(diff.added.len(), 2);
        assert_eq!(diff.removed.len(), 0);
    }

    #[test]
    fn test_diff_empty_new_all_removed() {
        let old = vec![
            make_row(&hash_bullet("Built API"), "Built API"),
            make_row(&hash_bullet("Led team"), "Led team"),
        ];
        let new: Vec<(String, Option<Uuid>)> = vec![];
        let diff = diff_bullets(&old, &new);
        assert_eq!(diff.unchanged.len(), 0);
        assert_eq!(diff.added.len(), 0);
        assert_eq!(diff.removed.len(), 2);
    }

    #[test]
    fn test_diff_all_unchanged() {
        let text1 = "Built API";
        let text2 = "Led team";
        let old = vec![
            make_row(&hash_bullet(text1), text1),
            make_row(&hash_bullet(text2), text2),
        ];
        let new = vec![
            (text1.to_string(), None),
            (text2.to_string(), None),
        ];
        let diff = diff_bullets(&old, &new);
        assert_eq!(diff.unchanged.len(), 2);
        assert_eq!(diff.added.len(), 0);
        assert_eq!(diff.removed.len(), 0);
    }

    #[test]
    fn test_diff_partial_change() {
        let text_keep = "Built API";
        let text_old = "Old bullet";
        let text_new = "New bullet";
        let old = vec![
            make_row(&hash_bullet(text_keep), text_keep),
            make_row(&hash_bullet(text_old), text_old),
        ];
        let new = vec![
            (text_keep.to_string(), None),
            (text_new.to_string(), None),
        ];
        let diff = diff_bullets(&old, &new);
        assert_eq!(diff.unchanged.len(), 1);
        assert!(diff.unchanged.contains(&hash_bullet(text_keep)));
        assert_eq!(diff.added.len(), 1);
        assert_eq!(diff.added[0].text, text_new);
        assert_eq!(diff.removed.len(), 1);
        assert_eq!(diff.removed[0], hash_bullet(text_old));
    }

    #[test]
    fn test_diff_empty_empty() {
        let old: Vec<PrepBulletRow> = vec![];
        let new: Vec<(String, Option<Uuid>)> = vec![];
        let diff = diff_bullets(&old, &new);
        assert_eq!(diff.unchanged.len(), 0);
        assert_eq!(diff.added.len(), 0);
        assert_eq!(diff.removed.len(), 0);
    }
}
