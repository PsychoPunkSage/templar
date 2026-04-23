//! Axum handlers for the Cover Letter API.
//!
//! POST /api/v1/cover-letters/generate  — synchronous generation
//! GET  /api/v1/cover-letters/:id        — fetch by ID
//! GET  /api/v1/cover-letters            — list by user_id (optionally filtered by resume_id)

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::cover_letter::{generator, CoverLetterRow, GenerateCoverLetterRequest};
use crate::errors::AppError;
use crate::state::AppState;

// ────────────────────────────────────────────────────────────────────────────
// POST /api/v1/cover-letters/generate
// ────────────────────────────────────────────────────────────────────────────

pub async fn handle_generate_cover_letter(
    State(state): State<AppState>,
    Json(request): Json<GenerateCoverLetterRequest>,
) -> Result<impl IntoResponse, AppError> {
    if request.jd_text.trim().is_empty() {
        return Err(AppError::Validation("jd_text cannot be empty".to_string()));
    }

    let cover_letter = generator::generate_cover_letter(&state, &request)
        .await
        .map_err(|e| AppError::Internal(e))?;

    // Link to project via resume_id if provided (fire-and-forget — non-fatal)
    if let Some(resume_id) = request.resume_id {
        let cl_id = cover_letter.id;
        let db = state.db.clone();
        tokio::spawn(async move {
            let _ = sqlx::query(
                "UPDATE cv_projects SET current_cover_letter_id = $1 WHERE current_resume_id = $2",
            )
            .bind(cl_id)
            .bind(resume_id)
            .execute(&db)
            .await;
        });
    }

    Ok((StatusCode::CREATED, Json(cover_letter)))
}

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/cover-letters/:id
// ────────────────────────────────────────────────────────────────────────────

pub async fn handle_get_cover_letter(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let row = sqlx::query_as::<_, CoverLetterRow>("SELECT * FROM cover_letters WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Cover letter {id} not found")))?;

    Ok(Json(row))
}

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/cover-letters?user_id=&resume_id=
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListCoverLettersQuery {
    pub user_id: Uuid,
    pub resume_id: Option<Uuid>,
}

pub async fn handle_list_cover_letters(
    State(state): State<AppState>,
    Query(q): Query<ListCoverLettersQuery>,
) -> Result<impl IntoResponse, AppError> {
    let rows = if let Some(rid) = q.resume_id {
        sqlx::query_as::<_, CoverLetterRow>(
            "SELECT * FROM cover_letters WHERE user_id = $1 AND resume_id = $2 ORDER BY created_at DESC",
        )
        .bind(q.user_id)
        .bind(rid)
        .fetch_all(&state.db)
        .await?
    } else {
        sqlx::query_as::<_, CoverLetterRow>(
            "SELECT * FROM cover_letters WHERE user_id = $1 ORDER BY created_at DESC",
        )
        .bind(q.user_id)
        .fetch_all(&state.db)
        .await?
    };

    Ok(Json(serde_json::json!({ "cover_letters": rows })))
}
