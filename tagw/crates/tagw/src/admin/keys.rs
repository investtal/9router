use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::auth::dashboard::AdminUser;
use crate::auth::member_key::{
    create_member_key, list_member_keys, revoke_member_key, MemberApiKeyPublic,
};
use crate::error::AppError;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub struct CreateKeyRequest {
    pub name: String,
}

/// Response for key creation — includes plaintext secret once.
#[derive(Debug, Serialize)]
pub struct CreateKeyResponse {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub created_at: String,
    /// Plaintext API key; shown only on create.
    pub key: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/keys", post(create_key).get(list_keys))
        .route("/api/admin/keys/{id}", delete(delete_key))
}

async fn create_key(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<CreateKeyRequest>,
) -> Result<Json<CreateKeyResponse>, AppError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AppError::BadRequest("name must not be empty".into()));
    }
    let (row, plaintext) = create_member_key(&state.db, name).map_err(AppError::Internal)?;
    // Point update first so auth stays consistent if full reload fails.
    state.cache.upsert(&row);
    if let Err(e) = state.cache.reload(&state.db) {
        tracing::warn!(error = %e, "config cache reload after create failed; upsert applied");
    }
    Ok(Json(CreateKeyResponse {
        id: row.id,
        name: row.name,
        key_prefix: row.key_prefix,
        created_at: row.created_at,
        key: plaintext,
    }))
}

async fn list_keys(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Vec<MemberApiKeyPublic>>, AppError> {
    let rows = list_member_keys(&state.db).map_err(AppError::Internal)?;
    let public: Vec<MemberApiKeyPublic> = rows.into_iter().map(Into::into).collect();
    Ok(Json(public))
}

async fn delete_key(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let revoked = revoke_member_key(&state.db, &id).map_err(AppError::Internal)?;
    if !revoked {
        return Err(AppError::NotFound(format!("member key {id}")));
    }
    // Point remove first so revoked keys stop authenticating if full reload fails.
    state.cache.remove_key(&id);
    if let Err(e) = state.cache.reload(&state.db) {
        tracing::warn!(error = %e, "config cache reload after revoke failed; remove_key applied");
    }
    Ok(StatusCode::NO_CONTENT)
}
