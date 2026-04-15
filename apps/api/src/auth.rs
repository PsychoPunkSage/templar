//! Clerk JWT verification and user resolution for Templar.
//!
//! # Design
//!
//! When `CLERK_JWKS_URL` is set, every request to `/api/v1/auth/me` verifies the
//! Bearer JWT against Clerk's published JWKS (RS256).  The `sub` claim (Clerk's
//! external user ID) is looked up in `users.external_id`; if not found, a new
//! user row is auto-created.
//!
//! When `CLERK_JWKS_URL` is **not** set (dev / test), the endpoint returns the
//! seed MVP UUID so the existing 223-test suite continues to pass unchanged.
//!
//! All other API endpoints remain unchanged — they still accept `user_id` from
//! body / query params.  Migrating them to use `AuthUser` is a follow-up task.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{extract::State, http::HeaderMap, Json};
use jsonwebtoken::{
    decode, decode_header,
    jwk::{AlgorithmParameters, JwkSet},
    DecodingKey, TokenData, Validation,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::errors::AppError;
use crate::state::AppState;

// ── Constants ────────────────────────────────────────────────────────────────

/// UUID of the seed dev/test user (migration 006_seed.sql).
const MVP_USER_ID: &str = "00000000-0000-0000-0000-000000000001";

// ── JWKS cache ───────────────────────────────────────────────────────────────

/// Thread-safe in-memory JWKS cache.
/// Maps `kid` → base64url-encoded (n, e) components for RSA-256 keys.
#[derive(Clone, Default)]
pub struct JwksCache(pub Arc<RwLock<HashMap<String, DecodingKey>>>);

impl JwksCache {
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(HashMap::new())))
    }
}

/// Fetches the JWKS from the given URL and builds DecodingKey objects for each
/// RSA key found.  Returns an empty map if the URL is unreachable (non-fatal at
/// startup — auth will 401 on first request, then retry).
pub async fn fetch_jwks(url: &str) -> HashMap<String, DecodingKey> {
    let client = reqwest::Client::new();
    match client.get(url).send().await {
        Err(e) => {
            tracing::warn!(url, error = %e, "Failed to fetch JWKS at startup");
            HashMap::new()
        }
        Ok(resp) => match resp.json::<JwkSet>().await {
            Err(e) => {
                tracing::warn!(url, error = %e, "Failed to parse JWKS response");
                HashMap::new()
            }
            Ok(jwk_set) => {
                let mut map = HashMap::new();
                for key in &jwk_set.keys {
                    let kid = match &key.common.key_id {
                        Some(k) => k.clone(),
                        None => continue,
                    };
                    if let AlgorithmParameters::RSA(rsa) = &key.algorithm {
                        match DecodingKey::from_rsa_components(&rsa.n, &rsa.e) {
                            Ok(dk) => {
                                map.insert(kid, dk);
                            }
                            Err(e) => {
                                tracing::warn!(kid, error = %e, "Failed to build DecodingKey from RSA components");
                            }
                        }
                    }
                }
                tracing::info!(url, count = map.len(), "JWKS loaded");
                map
            }
        },
    }
}

// ── JWT claims ───────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ClerkClaims {
    /// Clerk user ID (becomes `external_id` in our DB).
    sub: String,
}

// ── Internal helpers ─────────────────────────────────────────────────────────

/// Extracts the raw Bearer token from the `Authorization` header.
fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
}

/// Verifies a Clerk JWT against the cached JWKS.  On verification failure
/// (e.g. rotated key), re-fetches JWKS once and retries.
async fn verify_jwt(
    token: &str,
    cache: &JwksCache,
    jwks_url: &str,
) -> Result<TokenData<ClerkClaims>, AppError> {
    let header = decode_header(token)
        .map_err(|_| AppError::Validation("Invalid JWT header".to_string()))?;
    let kid = header
        .kid
        .ok_or_else(|| AppError::Validation("JWT missing kid".to_string()))?;

    // First attempt — use cached key if present
    {
        let guard = cache.0.read().await;
        if let Some(key) = guard.get(&kid) {
            let mut validation = Validation::new(jsonwebtoken::Algorithm::RS256);
            validation.validate_exp = true;
            if let Ok(data) = decode::<ClerkClaims>(token, key, &validation) {
                return Ok(data);
            }
        }
    }

    // Key not found or signature changed — refresh JWKS once and retry
    tracing::debug!(kid, "JWKS cache miss or verify failed — refreshing");
    let fresh = fetch_jwks(jwks_url).await;
    {
        let mut guard = cache.0.write().await;
        guard.extend(fresh);
    }

    let guard = cache.0.read().await;
    let key = guard.get(&kid).ok_or(AppError::Unauthorized)?;

    let mut validation = Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.validate_exp = true;
    decode::<ClerkClaims>(token, key, &validation).map_err(|_| AppError::Unauthorized)
}

/// Looks up a user by Clerk `external_id`, auto-creating if absent.
/// Returns the internal UUID.
async fn resolve_user(db: &sqlx::PgPool, external_id: &str) -> Result<Uuid, AppError> {
    // Fast path: user already exists
    let row: Option<(Uuid,)> =
        sqlx::query_as("SELECT id FROM users WHERE external_id = $1")
            .bind(external_id)
            .fetch_optional(db)
            .await
            .map_err(AppError::Database)?;

    if let Some((id,)) = row {
        return Ok(id);
    }

    // Auto-create — email is nullable after migration 015
    let (id,): (Uuid,) = sqlx::query_as(
        "INSERT INTO users (external_id, tier) VALUES ($1, 'free')
         ON CONFLICT (external_id) DO UPDATE SET external_id = EXCLUDED.external_id
         RETURNING id",
    )
    .bind(external_id)
    .fetch_one(db)
    .await
    .map_err(AppError::Database)?;

    tracing::info!(external_id, %id, "Auto-created user from Clerk sign-in");
    Ok(id)
}

// ── Handler ──────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct AuthMeResponse {
    pub user_id: Uuid,
}

/// GET /api/v1/auth/me
///
/// Resolves the caller's internal UUID:
/// - If `CLERK_JWKS_URL` is not configured → returns the seed MVP UUID (dev/test mode).
/// - Otherwise → verifies the Bearer JWT, looks up / auto-creates the user, returns UUID.
pub async fn handle_auth_me(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<AuthMeResponse>, AppError> {
    // Dev/test mode: auth not configured
    let jwks_url = match &state.config.clerk_jwks_url {
        None => {
            let id = Uuid::parse_str(MVP_USER_ID).expect("MVP_USER_ID is a valid UUID");
            return Ok(Json(AuthMeResponse { user_id: id }));
        }
        Some(url) => url.clone(),
    };

    // Auth mode: require Bearer token
    let token = extract_bearer(&headers).ok_or(AppError::Unauthorized)?;

    let claims = verify_jwt(token, &state.jwks_cache, &jwks_url)
        .await?
        .claims;

    let user_id = resolve_user(&state.db, &claims.sub).await?;

    Ok(Json(AuthMeResponse { user_id }))
}
