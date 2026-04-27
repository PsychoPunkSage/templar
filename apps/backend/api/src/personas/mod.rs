pub mod dedup;
pub mod handlers;
pub mod hash;
pub mod prompts;
pub mod suggestion_cache;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Deserialize)]
pub struct CreatePersonaRequest {
    pub user_id: Uuid,
    pub name: String,
    #[serde(default)]
    pub emphasized_tags: Vec<String>,
    #[serde(default)]
    pub suppressed_tags: Vec<String>,
    pub tone_preference: Option<String>,
    pub section_order: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct UpdatePersonaRequest {
    pub name: Option<String>,
    pub emphasized_tags: Option<Vec<String>>,
    pub suppressed_tags: Option<Vec<String>>,
    pub tone_preference: Option<String>,
    pub section_order: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonaSuggestion {
    pub name: String,
    pub emphasized_tags: Vec<String>,
    pub suppressed_tags: Vec<String>,
    pub tone_preference: Option<String>,
    pub reasoning: String,
}

/// Response envelope for GET /api/v1/personas/suggest.
/// Includes content hashes so the frontend can detect staleness without a round-trip.
#[derive(Debug, Serialize)]
pub struct SuggestPersonasResponse {
    pub suggestions: Vec<PersonaSuggestion>,
    pub context_hash: String,
    pub persona_hash: String,
}

/// Query params for GET /api/v1/personas/suggest.
/// ctx_hash + persona_hash are optional — sent only when the client has a cached result.
/// If both match the server-computed hashes, the handler returns 204 No Content (no LLM call).
#[derive(Debug, Deserialize)]
pub struct SuggestPersonasQuery {
    pub user_id: Uuid,
    pub ctx_hash: Option<String>,
    pub persona_hash: Option<String>,
}
