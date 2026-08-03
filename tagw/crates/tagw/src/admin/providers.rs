//! Admin CRUD for API-key providers and their accounts.
//!
//! - `GET /api/admin/providers` and `GET /api/providers` — any authenticated (redacted secrets)
//! - Mutating routes — admin only

use axum::extract::{Path, State};
use axum::routing::{get, patch, post};
use axum::{Json, Router};

use crate::auth::dashboard::{AdminUser, AuthUser};
use crate::error::AppError;
use crate::providers::api_key::{
    create_account, create_provider, list_providers, set_account_enabled, set_provider_enabled,
    AccountPublic, CreateAccountRequest, CreateProviderRequest, PatchEnabledRequest,
    ProviderPublic,
};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        // Viewer-readable list (redacted credentials).
        .route("/api/providers", get(list_providers_handler))
        .route(
            "/api/admin/providers",
            get(list_providers_handler).post(create_provider_handler),
        )
        .route(
            "/api/admin/providers/{id}",
            patch(patch_provider_handler),
        )
        .route(
            "/api/admin/providers/{id}/accounts",
            post(create_account_handler),
        )
        .route(
            "/api/admin/providers/{id}/accounts/{account_id}",
            patch(patch_account_handler),
        )
}

/// Reload config cache after provider/account mutation (keys + pools).
fn reload_cache(state: &AppState) {
    if let Err(e) = state.cache.reload(&state.db) {
        tracing::warn!(error = %e, "config cache reload after provider mutate failed");
    }
}

async fn list_providers_handler(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<Vec<ProviderPublic>>, AppError> {
    let rows = list_providers(&state.db).map_err(AppError::Internal)?;
    Ok(Json(rows))
}

async fn create_provider_handler(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(body): Json<CreateProviderRequest>,
) -> Result<Json<ProviderPublic>, AppError> {
    let row = create_provider(&state.db, &body).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("unsupported provider_type")
            || msg.contains("must not be empty")
            || msg.contains("required")
        {
            AppError::BadRequest(msg)
        } else {
            AppError::Internal(e)
        }
    })?;
    reload_cache(&state);
    Ok(Json(row))
}

async fn patch_provider_handler(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<String>,
    Json(body): Json<PatchEnabledRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let updated = set_provider_enabled(&state.db, &id, body.enabled).map_err(AppError::Internal)?;
    if !updated {
        return Err(AppError::NotFound(format!("provider {id}")));
    }
    reload_cache(&state);
    Ok(Json(serde_json::json!({
        "id": id,
        "enabled": body.enabled,
    })))
}

async fn create_account_handler(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(id): Path<String>,
    Json(body): Json<CreateAccountRequest>,
) -> Result<Json<AccountPublic>, AppError> {
    let row = create_account(&state.db, &id, &body).map_err(|e| {
        let msg = e.to_string();
        if msg.contains("not found") {
            AppError::NotFound(msg)
        } else if msg.contains("must not be empty")
            || msg.contains("required")
            || msg.contains("unsupported")
        {
            AppError::BadRequest(msg)
        } else {
            AppError::Internal(e)
        }
    })?;
    reload_cache(&state);
    Ok(Json(row))
}

async fn patch_account_handler(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path((id, account_id)): Path<(String, String)>,
    Json(body): Json<PatchEnabledRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let updated = set_account_enabled(&state.db, &id, &account_id, body.enabled)
        .map_err(AppError::Internal)?;
    if !updated {
        return Err(AppError::NotFound(format!(
            "account {account_id} under provider {id}"
        )));
    }
    reload_cache(&state);
    Ok(Json(serde_json::json!({
        "id": account_id,
        "provider_id": id,
        "enabled": body.enabled,
    })))
}
