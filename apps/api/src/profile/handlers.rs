use axum::{extract::{Query, State}, http::StatusCode, Json};
use serde::Deserialize;
use uuid::Uuid;

use crate::models::user::{ProfileLink, UserProfile};
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct UserIdQuery {
    pub user_id: Uuid,
}

#[derive(Debug, Deserialize)]
pub struct UpsertProfileRequest {
    pub user_id: Uuid,
    pub full_name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub location: Option<String>,
    pub links: Option<Vec<ProfileLink>>,
}

/// GET /api/v1/profile?user_id=<uuid>
/// Returns the user's profile. If no profile exists, returns empty defaults (not 404).
pub async fn handle_get_profile(
    State(state): State<AppState>,
    Query(q): Query<UserIdQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let row = sqlx::query_as::<_, UserProfile>(
        "SELECT * FROM user_profiles WHERE user_id = $1",
    )
    .bind(q.user_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    match row {
        Some(p) => Ok(Json(serde_json::json!({
            "full_name": p.full_name,
            "email": p.email,
            "phone": p.phone,
            "location": p.location,
            "links": p.links,
        }))),
        None => Ok(Json(serde_json::json!({
            "full_name": "",
            "email": "",
            "phone": "",
            "location": "",
            "links": [],
        }))),
    }
}

/// PUT /api/v1/profile
/// Creates or updates the user's profile (upsert by user_id).
pub async fn handle_upsert_profile(
    State(state): State<AppState>,
    Json(req): Json<UpsertProfileRequest>,
) -> Result<(StatusCode, Json<serde_json::Value>), (StatusCode, String)> {
    let links_json = serde_json::to_value(req.links.unwrap_or_default())
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let row = sqlx::query_as::<_, UserProfile>(
        r#"INSERT INTO user_profiles (user_id, full_name, email, phone, location, links)
           VALUES ($1, $2, $3, $4, $5, $6)
           ON CONFLICT (user_id) DO UPDATE SET
               full_name = EXCLUDED.full_name,
               email     = EXCLUDED.email,
               phone     = EXCLUDED.phone,
               location  = EXCLUDED.location,
               links     = EXCLUDED.links,
               updated_at = NOW()
           RETURNING *"#,
    )
    .bind(req.user_id)
    .bind(req.full_name.unwrap_or_default())
    .bind(req.email.unwrap_or_default())
    .bind(req.phone.unwrap_or_default())
    .bind(req.location.unwrap_or_default())
    .bind(links_json)
    .fetch_one(&state.db)
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    Ok((StatusCode::OK, Json(serde_json::json!({
        "full_name": row.full_name,
        "email": row.email,
        "phone": row.phone,
        "location": row.location,
        "links": row.links,
    }))))
}
