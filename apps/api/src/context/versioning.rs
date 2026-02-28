#![allow(dead_code)]

use anyhow::Result;
use aws_sdk_s3::primitives::ByteStream;
use sqlx::PgPool;
use tracing::info;
use uuid::Uuid;

use crate::models::context::{ContextEntryRow, ContextSnapshotRow};

pub struct ContextVersion {
    pub version: i32,
    pub s3_key: String,
    pub snapshot_id: Uuid,
}

/// Parameters for committing a new context entry version.
pub struct CommitParams<'a> {
    pub user_id: Uuid,
    pub entry_id: Uuid,
    pub entry_type: &'a str,
    pub data: &'a serde_json::Value,
    pub raw_text: Option<&'a str>,
    pub recency_score: f64,
    pub impact_score: f64,
    pub tags: &'a [String],
    pub flagged_evergreen: bool,
    pub contribution_type: &'a str,
}

/// Commits a new context entry as a versioned INSERT.
/// CRITICAL: This is append-only. Never UPDATE existing rows.
pub async fn commit_context_update(
    pool: &PgPool,
    s3: &aws_sdk_s3::Client,
    s3_bucket: &str,
    params: CommitParams<'_>,
) -> Result<ContextVersion> {
    let CommitParams {
        user_id,
        entry_id,
        entry_type,
        data,
        raw_text,
        recency_score,
        impact_score,
        tags,
        flagged_evergreen,
        contribution_type,
    } = params;
    // 1. Determine next version
    let current_max: Option<i32> =
        sqlx::query_scalar("SELECT MAX(version) FROM context_entries WHERE user_id = $1")
            .bind(user_id)
            .fetch_one(pool)
            .await?;
    let new_version = current_max.unwrap_or(0) + 1;

    // 2. Append-only INSERT
    sqlx::query(
        r#"
        INSERT INTO context_entries
            (user_id, entry_id, version, entry_type, data, raw_text,
             recency_score, impact_score, tags, flagged_evergreen, contribution_type)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        "#,
    )
    .bind(user_id)
    .bind(entry_id)
    .bind(new_version)
    .bind(entry_type)
    .bind(data)
    .bind(raw_text)
    .bind(recency_score)
    .bind(impact_score)
    .bind(tags)
    .bind(flagged_evergreen)
    .bind(contribution_type)
    .execute(pool)
    .await?;

    info!("Inserted context entry {entry_id} version {new_version} for user {user_id}");

    // 3. Render all current entries to markdown
    let all_entries = get_current_entries(pool, user_id).await?;
    let md_content = render_context_to_md(user_id, &all_entries);

    // 4. Upload markdown snapshot to S3
    let s3_key = format!("contexts/{}/v{}.md", user_id, new_version);
    s3.put_object()
        .bucket(s3_bucket)
        .key(&s3_key)
        .body(ByteStream::from(md_content.into_bytes()))
        .content_type("text/markdown")
        .send()
        .await
        .map_err(|e| anyhow::anyhow!("S3 upload failed: {e}"))?;

    info!("Uploaded context snapshot to s3://{}/{}", s3_bucket, s3_key);

    // 5. Record snapshot
    let snapshot_id = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO context_snapshots (id, user_id, version, s3_key) VALUES ($1, $2, $3, $4)",
    )
    .bind(snapshot_id)
    .bind(user_id)
    .bind(new_version)
    .bind(&s3_key)
    .execute(pool)
    .await?;

    Ok(ContextVersion {
        version: new_version,
        s3_key,
        snapshot_id,
    })
}

/// Returns the most recent version of each entry for a user.
pub async fn get_current_entries(pool: &PgPool, user_id: Uuid) -> Result<Vec<ContextEntryRow>> {
    Ok(sqlx::query_as::<_, ContextEntryRow>(
        r#"
        SELECT DISTINCT ON (entry_id) *
        FROM context_entries
        WHERE user_id = $1
        ORDER BY entry_id, version DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

/// Returns all entries as of a specific version number.
pub async fn get_entries_at_version(
    pool: &PgPool,
    user_id: Uuid,
    version: i32,
) -> Result<Vec<ContextEntryRow>> {
    Ok(sqlx::query_as::<_, ContextEntryRow>(
        r#"
        SELECT DISTINCT ON (entry_id) *
        FROM context_entries
        WHERE user_id = $1 AND version <= $2
        ORDER BY entry_id, version DESC
        "#,
    )
    .bind(user_id)
    .bind(version)
    .fetch_all(pool)
    .await?)
}

/// Returns all context snapshot versions for a user.
pub async fn get_version_history(pool: &PgPool, user_id: Uuid) -> Result<Vec<ContextSnapshotRow>> {
    Ok(sqlx::query_as::<_, ContextSnapshotRow>(
        "SELECT * FROM context_snapshots WHERE user_id = $1 ORDER BY version ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await?)
}

/// Renders all context entries as a structured markdown document.
pub fn render_context_to_md(user_id: Uuid, entries: &[ContextEntryRow]) -> String {
    let mut md = format!("# Context Snapshot — User {}\n\n", user_id);
    let sections = [
        "experience",
        "education",
        "project",
        "skill",
        "publication",
        "open_source",
        "certification",
        "award",
        "extracurricular",
    ];
    for section in sections {
        let section_entries: Vec<_> = entries.iter().filter(|e| e.entry_type == section).collect();
        if section_entries.is_empty() {
            continue;
        }
        let title = section.replace('_', " ");
        let title = title
            .split_whitespace()
            .map(|w| {
                let mut c = w.chars();
                match c.next() {
                    None => String::new(),
                    Some(f) => f.to_uppercase().to_string() + c.as_str(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");
        md.push_str(&format!("## {title}\n\n"));
        for entry in section_entries {
            md.push_str(&format!("### Entry: {}\n", entry.entry_id));
            md.push_str(&format!("- **Version:** {}\n", entry.version));
            md.push_str(&format!("- **Recency:** {:.2}\n", entry.recency_score));
            md.push_str(&format!("- **Impact:** {:.2}\n", entry.impact_score));
            md.push_str(&format!(
                "- **Contribution:** {}\n",
                entry.contribution_type
            ));
            if !entry.tags.is_empty() {
                md.push_str(&format!("- **Tags:** {}\n", entry.tags.join(", ")));
            }
            if let Ok(data_str) = serde_json::to_string_pretty(&entry.data) {
                md.push_str("```json\n");
                md.push_str(&data_str);
                md.push_str("\n```\n");
            }
            md.push('\n');
        }
    }
    md
}
