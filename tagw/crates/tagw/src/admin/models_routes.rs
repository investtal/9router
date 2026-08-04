//! Models catalog API (dashboard) + OpenAI `/v1/models` (member key).

use axum::extract::State;
use axum::http::HeaderMap;
use axum::routing::get;
use axum::{Json, Router};

use crate::auth::dashboard::AuthUser;
use crate::error::AppError;
use crate::models_catalog::{list_available_models, to_openai_models_list, ModelEntry};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/models", get(api_models))
        .route("/v1/models", get(v1_models))
}

/// Dashboard list (session auth) — full `ModelEntry` objects (`glm/glm-5.2`, …).
async fn api_models(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<ModelEntry>>, AppError> {
    let entries = list_available_models(&state.db, &state.cache)?;
    Ok(Json(entries))
}

/// OpenAI-compatible model list (member API key).
async fn v1_models(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<crate::models_catalog::OpenAiModelsList>, AppError> {
    let val = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(AppError::Unauthorized)?;
    let token = val
        .strip_prefix("Bearer ")
        .or_else(|| val.strip_prefix("bearer "))
        .unwrap_or(val)
        .trim();
    if token.is_empty() || state.cache.authenticate_bearer(token).is_none() {
        return Err(AppError::Unauthorized);
    }
    let entries = list_available_models(&state.db, &state.cache)?;
    Ok(Json(to_openai_models_list(&entries)))
}
