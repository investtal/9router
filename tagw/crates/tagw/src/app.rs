use axum::routing::{any, get};
use axum::Router;

use crate::admin;
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
        .merge(admin::keys::router())
        // OpenAI-compatible passthrough: POST /v1/chat/completions and other /v1/*
        .route("/v1/{*path}", any(proxy::openai::proxy_openai))
        .with_state(state)
}
