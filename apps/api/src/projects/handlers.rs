//! Axum handlers for the cv_projects API.
//!
//! GET    /api/v1/projects?user_id={uuid}  — list user's projects
//! POST   /api/v1/projects                 — create a new project
//! GET    /api/v1/projects/:id             — fetch one project
//! PATCH  /api/v1/projects/:id             — partial update
//! DELETE /api/v1/projects/:id             — hard delete

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::errors::AppError;
use crate::projects::{CreateProjectRequest, CvProjectRow, UpdateProjectRequest};
use crate::state::AppState;

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/projects?user_id={uuid}
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct ListProjectsQuery {
    pub user_id: Uuid,
}

/// Returns all projects for a user, ordered by `updated_at DESC`.
///
/// Performance: single indexed query — `idx_cv_projects_user_id` covers
/// (user_id, updated_at DESC), so no sort happens at runtime.
pub async fn handle_list_projects(
    Query(q): Query<ListProjectsQuery>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let projects = sqlx::query_as::<_, CvProjectRow>(
        "SELECT * FROM cv_projects WHERE user_id = $1 ORDER BY updated_at DESC",
    )
    .bind(q.user_id)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(serde_json::json!({ "projects": projects })))
}

// ────────────────────────────────────────────────────────────────────────────
// POST /api/v1/projects
// ────────────────────────────────────────────────────────────────────────────

/// Creates a new project for the given user.
///
/// Validates `template_id` against the in-memory TemplateCache (no DB query).
/// Returns 400 if the template_id is unknown — we do NOT create projects for
/// non-existent templates because the render worker would have no LaTeX to use.
pub async fn handle_create_project(
    State(state): State<AppState>,
    Json(body): Json<CreateProjectRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Validate template_id exists in our loaded cache.
    // This is a read lock on the cache — lightweight, no DB involved.
    {
        let cache = state.template_cache.read().await;
        if !cache.contains_key(&body.template_id) {
            return Err(AppError::Validation(format!(
                "Unknown template_id '{}'. Available templates: {}",
                body.template_id,
                cache.keys().cloned().collect::<Vec<_>>().join(", ")
            )));
        }
    }

    if body.name.trim().is_empty() {
        return Err(AppError::Validation(
            "Project name must not be empty".to_string(),
        ));
    }

    let project = sqlx::query_as::<_, CvProjectRow>(
        r#"INSERT INTO cv_projects (user_id, name, template_id)
           VALUES ($1, $2, $3)
           RETURNING *"#,
    )
    .bind(body.user_id)
    .bind(body.name.trim())
    .bind(&body.template_id)
    .fetch_one(&state.db)
    .await?;

    Ok((StatusCode::CREATED, Json(project)))
}

// ────────────────────────────────────────────────────────────────────────────
// GET /api/v1/projects/:id
// ────────────────────────────────────────────────────────────────────────────

/// Returns a single project by ID.
pub async fn handle_get_project(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let project = sqlx::query_as::<_, CvProjectRow>("SELECT * FROM cv_projects WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(AppError::NotFound(format!("Project {} not found", id)))?;

    Ok(Json(project))
}

// ────────────────────────────────────────────────────────────────────────────
// PATCH /api/v1/projects/:id
// ────────────────────────────────────────────────────────────────────────────

/// Partially updates a project: name, template_id, and/or current_resume_id.
///
/// Only provided fields are updated — absent fields are left unchanged.
/// Uses a SQL COALESCE pattern to handle partial updates without multiple
/// round-trips or conditional query building.
pub async fn handle_update_project(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(body): Json<UpdateProjectRequest>,
) -> Result<impl IntoResponse, AppError> {
    // Validate new template_id if provided
    if let Some(ref tid) = body.template_id {
        let cache = state.template_cache.read().await;
        if !cache.contains_key(tid) {
            return Err(AppError::Validation(format!("Unknown template_id '{tid}'")));
        }
    }

    // COALESCE($2, name): if $2 IS NULL (not provided), keep existing value.
    // This is simpler than building a dynamic SET clause and avoids multiple
    // DB round-trips for a partial update.
    let project = sqlx::query_as::<_, CvProjectRow>(
        r#"UPDATE cv_projects
           SET name              = COALESCE($2, name),
               template_id       = COALESCE($3, template_id),
               current_resume_id = COALESCE($4, current_resume_id),
               updated_at        = NOW()
           WHERE id = $1
           RETURNING *"#,
    )
    .bind(id)
    .bind(body.name.as_deref())
    .bind(body.template_id.as_deref())
    .bind(body.current_resume_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(AppError::NotFound(format!("Project {} not found", id)))?;

    Ok(Json(project))
}

// ────────────────────────────────────────────────────────────────────────────
// DELETE /api/v1/projects/:id
// ────────────────────────────────────────────────────────────────────────────

/// Hard-deletes a project row. Resume rows are NOT deleted:
/// the FK on cv_projects.current_resume_id is ON DELETE SET NULL (not CASCADE).
/// This means deleting a project doesn't destroy the user's generated resume.
pub async fn handle_delete_project(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<impl IntoResponse, AppError> {
    let rows = sqlx::query("DELETE FROM cv_projects WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await?
        .rows_affected();

    if rows == 0 {
        return Err(AppError::NotFound(format!("Project {} not found", id)));
    }

    Ok(StatusCode::NO_CONTENT)
}
