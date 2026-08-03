//! Loopback OAuth callbacks for public clients that only allow fixed redirect URIs.
//!
//! | Provider | Registered redirect |
//! |----------|---------------------|
//! | Codex    | `http://localhost:1455/auth/callback` |
//! | xAI      | `http://127.0.0.1:56121/callback` |
//!
//! These cannot be the gateway's `:20129/api/oauth/...` path — IdPs reject unregistered URIs.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use tokio::net::TcpListener;

use crate::error::AppError;
use crate::state::AppState;

use super::{dashboard_base_from_state, oauth_result_html};
use super::refresh::provider_by_id;
use super::save_oauth_account;

/// Fixed loopback registration used by a public OAuth client.
#[derive(Clone, Copy, Debug)]
pub struct LoopbackSpec {
    pub host: &'static str,
    pub port: u16,
    pub path: &'static str,
}

impl LoopbackSpec {
    pub fn redirect_uri(self) -> String {
        format!("http://{}:{}{}", self.host, self.port, self.path)
    }

    pub fn bind_addr(self) -> String {
        // Bind on all interfaces for the port; redirect_uri host still matches registry.
        format!("127.0.0.1:{}", self.port)
    }
}

/// Codex CLI public client — OpenAI only allows this exact redirect.
pub const CODEX_LOOPBACK: LoopbackSpec = LoopbackSpec {
    host: "localhost",
    port: 1455,
    path: "/auth/callback",
};

/// xAI / Grok CLI public client.
pub const XAI_LOOPBACK: LoopbackSpec = LoopbackSpec {
    host: "127.0.0.1",
    port: 56121,
    path: "/callback",
};

/// Return fixed loopback redirect for providers that require it.
pub fn loopback_for_provider(provider: &str) -> Option<LoopbackSpec> {
    match provider {
        "codex" => Some(CODEX_LOOPBACK),
        "xai" => Some(XAI_LOOPBACK),
        _ => None,
    }
}

/// Ports already serving a loopback OAuth catcher (process-wide).
pub type BoundPorts = Arc<Mutex<HashSet<u16>>>;

pub fn new_bound_ports() -> BoundPorts {
    Arc::new(Mutex::new(HashSet::new()))
}

/// Ensure a loopback HTTP server is listening for this provider's registered URI.
///
/// Safe to call multiple times; only binds once per port.
pub async fn ensure_loopback_server(
    state: AppState,
    provider: &str,
    bound: &BoundPorts,
) -> Result<LoopbackSpec, AppError> {
    let spec = loopback_for_provider(provider).ok_or_else(|| {
        AppError::BadRequest(format!("provider '{provider}' does not use loopback OAuth"))
    })?;

    {
        let g = bound.lock().expect("oauth loopback ports lock");
        if g.contains(&spec.port) {
            return Ok(spec);
        }
    }

    let listener = TcpListener::bind(spec.bind_addr()).await.map_err(|e| {
        AppError::Internal(anyhow::anyhow!(
            "cannot bind OAuth loopback {}:{} — is another process using it? ({e})",
            spec.host,
            spec.port
        ))
    })?;

    let app_state = state.clone();
    let path = spec.path.to_string();
    let provider_owned = provider.to_string();

    // Catch-all under path: handle GET with query string.
    let app = Router::new()
        .route(
            &path,
            get(move |Query(q): Query<LoopbackQuery>, State(st): State<AppState>| {
                let prov = provider_owned.clone();
                async move { handle_loopback_callback(st, &prov, q).await }
            }),
        )
        .with_state(app_state);

    {
        let mut g = bound.lock().expect("oauth loopback ports lock");
        g.insert(spec.port);
    }

    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!(error = %e, port = spec.port, "oauth loopback server exited");
        }
    });

    tracing::info!(
        port = spec.port,
        path = %spec.path,
        redirect_uri = %spec.redirect_uri(),
        "oauth loopback server listening"
    );

    Ok(spec)
}

#[derive(Debug, Deserialize)]
struct LoopbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

async fn handle_loopback_callback(
    state: AppState,
    provider: &str,
    q: LoopbackQuery,
) -> Response {
    match complete_oauth_callback(&state, provider, q).await {
        Ok(html) => Html(html).into_response(),
        Err(e) => {
            let body = format!(
                "<p>{}</p><p>Retry <b>Connect</b> from the dashboard if needed.</p>",
                html_escape(&e.to_string())
            );
            let base = dashboard_base_from_state(&state);
            Html(oauth_result_html("OAuth failed", &body, &base)).into_response()
        }
    }
}

async fn complete_oauth_callback(
    state: &AppState,
    provider: &str,
    q: LoopbackQuery,
) -> Result<String, AppError> {
    if let Some(err) = q.error {
        let desc = q.error_description.unwrap_or_default();
        return Err(AppError::BadRequest(format!("{err}: {desc}")));
    }
    let code = q
        .code
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("missing code".into()))?;
    let state_param = q
        .state
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest("missing state".into()))?;

    let pending = {
        let map = state.oauth_pending.clone();
        let mut guard = map.lock().expect("oauth pending lock");
        guard.remove(&state_param)
    }
    .ok_or_else(|| AppError::BadRequest("unknown or expired OAuth state".into()))?;

    if pending.provider != provider {
        return Err(AppError::BadRequest(format!(
            "provider mismatch: start={} callback={provider}",
            pending.provider
        )));
    }

    let http = state.http_client.clone();
    let impl_ = provider_by_id(provider, http)
        .ok_or_else(|| AppError::NotFound(format!("unknown oauth provider '{provider}'")))?;

    let tokens = impl_
        .exchange_code(&code, &pending.pkce)
        .await
        .map_err(|e| AppError::Upstream(format!("oauth exchange failed: {e}")))?;

    let account_id = save_oauth_account(&state.db, provider, &tokens, impl_.default_base_url())
        .map_err(AppError::Internal)?;

    if let Err(e) = state.cache.reload(&state.db) {
        tracing::warn!(error = %e, "cache reload after loopback oauth failed");
    }

    let body = format!(
        "<p>Provider <b>{provider}</b> account <code>{account_id}</code> saved.</p>"
    );
    let base = dashboard_base_from_state(state);
    Ok(oauth_result_html("Connected", &body, &base))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
