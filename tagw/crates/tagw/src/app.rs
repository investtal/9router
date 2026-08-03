use axum::routing::{any, get, post};
use axum::Router;

use crate::admin;
use crate::auth::dashboard;
use crate::auth::oidc;
use crate::oauth;
use crate::proxy;
use crate::state::AppState;

pub fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/readyz",
            get(|axum::extract::State(s): axum::extract::State<AppState>| async move {
                if s.is_ready() {
                    (axum::http::StatusCode::OK, "ready")
                } else {
                    (axum::http::StatusCode::SERVICE_UNAVAILABLE, "not ready")
                }
            }),
        )
        .merge(dashboard::router())
        .merge(oidc::router())
        .merge(admin::keys::router())
        .merge(admin::providers::router())
        .merge(admin::users::router())
        .merge(admin::usage_routes::router())
        .merge(oauth::router())
        // Anthropic Messages (Claude Code) — registered before OpenAI catch-all.
        .route("/v1/messages", post(proxy::anthropic::proxy_anthropic))
        .route(
            "/v1/messages/count_tokens",
            post(proxy::anthropic::proxy_anthropic),
        )
        // OpenAI-compatible passthrough: POST /v1/chat/completions and other /v1/*
        .route("/v1/{*path}", any(proxy::openai::proxy_openai))
        .with_state(state)
}
