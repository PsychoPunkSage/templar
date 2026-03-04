use axum::{
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    Json,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::context::batch;
use crate::context::completeness::compute_completeness_report;
use crate::context::extractor;
use crate::context::ingest::{
    confirm_ingest, parse_and_validate, IngestConfirmRequest, IngestConfirmResponse,
    IngestPreviewResponse, IngestRequest,
};
use crate::context::splitter::split_entries;
use crate::context::versioning::{
    get_current_entries, get_entries_at_version, get_version_history,
};
use crate::errors::AppError;
use crate::models::context::{ContextEntryRow, ContextSnapshotRow};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct UserIdQuery {
    pub user_id: Uuid,
}

#[derive(Serialize)]
pub struct ContextListResponse {
    pub entries: Vec<ContextEntryRow>,
    pub completeness: crate::context::completeness::CompletenessReport,
}

/// POST /api/v1/context/ingest
pub async fn handle_ingest(
    State(state): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> Result<Json<IngestPreviewResponse>, AppError> {
    let preview = parse_and_validate(&req.raw_text, &state.llm, &state.db, req.user_id).await?;
    if !preview.impact_validation.passed {
        return Err(AppError::UnprocessableEntity(
            serde_json::to_string(&preview).unwrap_or_default(),
        ));
    }
    Ok(Json(preview))
}

/// POST /api/v1/context/ingest/confirm
pub async fn handle_ingest_confirm(
    State(state): State<AppState>,
    Json(req): Json<IngestConfirmRequest>,
) -> Result<Json<IngestConfirmResponse>, AppError> {
    let response = confirm_ingest(&state.db, &state.s3, &state.config.s3_bucket, &req).await?;
    Ok(Json(response))
}

/// GET /api/v1/context
pub async fn handle_get_context(
    State(state): State<AppState>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<ContextListResponse>, AppError> {
    let entries = get_current_entries(&state.db, params.user_id).await?;
    let completeness = compute_completeness_report(&entries);
    Ok(Json(ContextListResponse {
        entries,
        completeness,
    }))
}

/// GET /api/v1/context/health
pub async fn handle_context_health(
    State(state): State<AppState>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<crate::context::completeness::CompletenessReport>, AppError> {
    let entries = get_current_entries(&state.db, params.user_id).await?;
    Ok(Json(compute_completeness_report(&entries)))
}

/// GET /api/v1/context/history
pub async fn handle_context_history(
    State(state): State<AppState>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<Vec<ContextSnapshotRow>>, AppError> {
    let history = get_version_history(&state.db, params.user_id).await?;
    Ok(Json(history))
}

/// GET /api/v1/context/version/:v
pub async fn handle_get_version(
    State(state): State<AppState>,
    Path(v): Path<i32>,
    Query(params): Query<UserIdQuery>,
) -> Result<Json<Vec<ContextEntryRow>>, AppError> {
    let entries = get_entries_at_version(&state.db, params.user_id, v).await?;
    Ok(Json(entries))
}

#[derive(Deserialize)]
pub struct EvergreenToggle {
    pub flagged_evergreen: bool,
    pub user_id: Uuid,
}

/// PATCH /api/v1/context/entries/:id/evergreen
pub async fn handle_toggle_evergreen(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(req): Json<EvergreenToggle>,
) -> Result<StatusCode, AppError> {
    let existing: Option<ContextEntryRow> = sqlx::query_as(
        "SELECT * FROM context_entries WHERE entry_id = $1 AND user_id = $2 ORDER BY version DESC LIMIT 1",
    )
    .bind(id)
    .bind(req.user_id)
    .fetch_optional(&state.db)
    .await?;

    let existing = existing.ok_or_else(|| AppError::NotFound(format!("Entry {id} not found")))?;

    // Append-only: INSERT a new version with updated evergreen flag, never UPDATE
    sqlx::query(
        r#"
        INSERT INTO context_entries
            (user_id, entry_id, version, entry_type, data, raw_text,
             recency_score, impact_score, tags, flagged_evergreen, contribution_type)
        SELECT user_id, entry_id, $1, entry_type, data, raw_text,
               recency_score, impact_score, tags, $2, contribution_type
        FROM context_entries
        WHERE entry_id = $3 AND user_id = $4
        ORDER BY version DESC
        LIMIT 1
        "#,
    )
    .bind(existing.version + 1)
    .bind(req.flagged_evergreen)
    .bind(id)
    .bind(req.user_id)
    .execute(&state.db)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

// ────────────────────────────────────────────────────────────────────────────
// Batch ingestion handlers (Phase: async batch pipeline)
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct IngestBatchRequest {
    pub user_id: Uuid,
    pub raw_text: String,
}

#[derive(Debug, Serialize)]
pub struct BatchStartResponse {
    pub batch_id: Uuid,
    pub entry_count: usize,
    pub status: &'static str,
}

/// POST /api/v1/context/ingest/batch
///
/// Accepts a raw text document (may contain multiple entries separated by `\n---`),
/// splits it into individual entries, stores them in the DB, and enqueues all
/// item IDs to the Redis ingest queue. Returns immediately with a `batch_id`.
///
/// The client should poll `GET /api/v1/context/ingest/batch/:id` for progress.
#[tracing::instrument(skip(state, req), fields(user_id = %req.user_id, text_len = req.raw_text.len()))]
pub async fn handle_ingest_batch(
    State(state): State<AppState>,
    Json(req): Json<IngestBatchRequest>,
) -> Result<Json<BatchStartResponse>, AppError> {
    tracing::info!("batch text ingest requested");

    let entries = split_entries(&req.raw_text)?;
    let entry_count = entries.len();
    tracing::info!(entry_count, "split entries from raw text");

    let batch_id =
        batch::create_batch(&state.db, req.user_id, "text", None, &entries)
            .await
            .map_err(AppError::Internal)?;

    let item_ids = batch::get_item_ids(&state.db, batch_id)
        .await
        .map_err(AppError::Internal)?;

    batch::enqueue_batch_items(&state.redis, item_ids)
        .await
        .map_err(AppError::Internal)?;

    tracing::info!(%batch_id, entry_count, "text batch enqueued");

    Ok(Json(BatchStartResponse {
        batch_id,
        entry_count,
        status: "queued",
    }))
}

/// POST /api/v1/context/ingest/upload
///
/// Accepts a multipart form with fields:
/// - `user_id` (UUID string)
/// - `file` (binary — .md, .txt, or .pdf, max 10 MB)
///
/// Extracts text, splits into entries, stores in DB, enqueues to Redis.
/// Returns immediately with a `batch_id`.
#[tracing::instrument(skip(state, multipart))]
pub async fn handle_ingest_upload(
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    tracing::info!("file upload ingest requested");

    let mut user_id: Option<Uuid> = None;
    let mut file_data: Option<(String, Vec<u8>)> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::Validation(format!("Multipart error: {e}")))?
    {
        match field.name() {
            Some("user_id") => {
                let text = field
                    .text()
                    .await
                    .map_err(|e| AppError::Validation(format!("user_id field error: {e}")))?;
                user_id = Some(text.trim().parse::<Uuid>().map_err(|_| {
                    AppError::Validation("Invalid user_id: must be a valid UUID".into())
                })?);
            }
            Some("file") => {
                let filename = field.file_name().unwrap_or("upload.md").to_string();
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::Validation(format!("File read error: {e}")))?;
                file_data = Some((filename, bytes.to_vec()));
            }
            _ => {
                // Ignore unknown fields
            }
        }
    }

    let user_id =
        user_id.ok_or_else(|| AppError::Validation("Missing required field: user_id".into()))?;
    let (filename, bytes) = file_data
        .ok_or_else(|| AppError::Validation("Missing required field: file".into()))?;

    tracing::info!(
        %user_id,
        filename = %filename,
        file_size = bytes.len(),
        "extracting text from uploaded file"
    );

    let raw_text = extractor::extract_text(&filename, &bytes)?;
    let entries = split_entries(&raw_text)?;
    let entry_count = entries.len();

    let batch_id =
        batch::create_batch(&state.db, user_id, "file", Some(&filename), &entries)
            .await
            .map_err(AppError::Internal)?;

    let item_ids = batch::get_item_ids(&state.db, batch_id)
        .await
        .map_err(AppError::Internal)?;

    batch::enqueue_batch_items(&state.redis, item_ids)
        .await
        .map_err(AppError::Internal)?;

    tracing::info!(%batch_id, entry_count, filename = %filename, "file batch enqueued");

    Ok(Json(serde_json::json!({
        "batch_id": batch_id,
        "entry_count": entry_count,
        "filename": filename,
        "status": "queued"
    })))
}

/// GET /api/v1/context/ingest/batch/:id
///
/// Returns the current status of a batch including per-item progress.
/// Poll this endpoint at 2-second intervals until `status == "done"`.
#[tracing::instrument(skip(state))]
pub async fn handle_batch_status(
    State(state): State<AppState>,
    Path(batch_id): Path<Uuid>,
) -> Result<Json<batch::BatchStatusResponse>, AppError> {
    let status = batch::get_batch_status(&state.db, batch_id)
        .await
        .map_err(AppError::Internal)?
        .ok_or_else(|| AppError::NotFound(format!("Batch {batch_id} not found")))?;

    Ok(Json(status))
}
