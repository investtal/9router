//! OAuth connect flows + token refresh for gateway accounts.
//!
//! Providers: codex (full PKCE), claude, antigravity, xai, kimi.
//! Routes: `GET /api/oauth/:provider/start`, `GET /api/oauth/:provider/callback`.

pub mod antigravity;
pub mod claude;
pub mod codex;
pub mod kimi;
pub mod pkce;
pub mod refresh;
pub mod types;
pub mod xai;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use rusqlite::params;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::auth::dashboard::AdminUser;
use crate::cache::{CachedAccount, ConfigCache};
use crate::db::Db;
use crate::error::AppError;
use crate::router::AccountRef;
use crate::state::{AppState, ANTHROPIC_POOL_KEY, OPENAI_COMPAT_POOL_KEY};

pub use refresh::{
    ensure_access_token, ensure_access_token_with_client, provider_by_id, spawn_oauth_refresh_loop,
};
pub use types::{
    OAuthCredentials, OAuthProvider, Pkce, TokenSet, ACCESS_TOKEN_REFRESH_SKEW_SECS,
    BACKGROUND_REFRESH_INTERVAL_SECS, BACKGROUND_REFRESH_LEAD_SECS,
};

use pkce::generate_pkce;
use refresh::PendingMap;
use types::PendingOAuth;

/// Max age for in-memory PKCE start sessions.
const PENDING_TTL: Duration = Duration::from_secs(10 * 60);

/// Supported OAuth provider ids (start/callback).
pub const OAUTH_PROVIDER_IDS: &[&str] = &["codex", "claude", "antigravity", "xai", "kimi"];

/// Axum routes for OAuth connect.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/oauth/{provider}/start", get(oauth_start))
        .route("/api/oauth/{provider}/callback", get(oauth_callback))
}

#[derive(Debug, Deserialize)]
struct StartQuery {
    /// Absolute redirect_uri registered with the IdP. Defaults to this host's callback.
    redirect_uri: Option<String>,
    /// When true (default), HTTP 302 to the authorize URL. When false, JSON body.
    #[serde(default = "default_true")]
    redirect: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

fn pending_map(state: &AppState) -> PendingMap {
    state.oauth_pending.clone()
}

fn resolve_redirect_uri(
    state: &AppState,
    provider: &str,
    headers: &axum::http::HeaderMap,
    explicit: Option<String>,
) -> String {
    if let Some(u) = explicit.filter(|s| !s.trim().is_empty()) {
        return u;
    }
    if let Some(base) = state.public_base.as_deref() {
        return format!(
            "{}/api/oauth/{}/callback",
            base.trim_end_matches('/'),
            provider
        );
    }
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("127.0.0.1:20128");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    format!("{scheme}://{host}/api/oauth/{provider}/callback")
}

fn purge_stale_pending(map: &mut HashMap<String, PendingOAuth>) {
    let cutoff = Utc::now() - chrono::Duration::from_std(PENDING_TTL).unwrap_or_default();
    map.retain(|_, p| p.created_at > cutoff);
}

async fn oauth_start(
    State(state): State<AppState>,
    _admin: AdminUser,
    Path(provider): Path<String>,
    Query(q): Query<StartQuery>,
    headers: axum::http::HeaderMap,
) -> Result<Response, AppError> {
    let provider = provider.to_ascii_lowercase();
    if !OAUTH_PROVIDER_IDS.contains(&provider.as_str()) {
        return Err(AppError::NotFound(format!("unknown oauth provider '{provider}'")));
    }
    let http = state.http_client.clone();
    let impl_ = provider_by_id(&provider, http)
        .ok_or_else(|| AppError::NotFound(format!("unknown oauth provider '{provider}'")))?;

    let redirect_uri = resolve_redirect_uri(&state, &provider, &headers, q.redirect_uri);
    let pkce = generate_pkce(redirect_uri);
    let authorize_url = impl_.authorize_url(&pkce);

    {
        let map = pending_map(&state);
        let mut guard = map.lock().expect("oauth pending lock");
        purge_stale_pending(&mut guard);
        guard.insert(
            pkce.state.clone(),
            PendingOAuth {
                provider: provider.clone(),
                pkce: pkce.clone(),
                created_at: Utc::now(),
            },
        );
    }

    if q.redirect {
        Ok(Redirect::temporary(&authorize_url).into_response())
    } else {
        Ok(Json(json!({
            "provider": provider,
            "authorize_url": authorize_url,
            "state": pkce.state,
            "redirect_uri": pkce.redirect_uri,
        }))
        .into_response())
    }
}

async fn oauth_callback(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Query(q): Query<CallbackQuery>,
) -> Result<Response, AppError> {
    let provider = provider.to_ascii_lowercase();
    if let Some(err) = q.error {
        let desc = q.error_description.unwrap_or_default();
        return Ok(Html(format!(
            "<html><body><h1>OAuth error</h1><p>{err}</p><p>{desc}</p></body></html>"
        ))
        .into_response());
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
        let map = pending_map(&state);
        let mut guard = map.lock().expect("oauth pending lock");
        purge_stale_pending(&mut guard);
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
    let impl_ = provider_by_id(&provider, http)
        .ok_or_else(|| AppError::NotFound(format!("unknown oauth provider '{provider}'")))?;

    let tokens = impl_
        .exchange_code(&code, &pending.pkce)
        .await
        .map_err(|e| AppError::Upstream(format!("oauth exchange failed: {e}")))?;

    let account_id = save_oauth_account(&state.db, &provider, &tokens, impl_.default_base_url())
        .map_err(AppError::Internal)?;

    if let Err(e) = state.cache.reload(&state.db) {
        tracing::warn!(error = %e, "cache reload after oauth callback failed");
    }

    Ok(Html(format!(
        "<html><body><h1>Connected</h1><p>Provider <b>{provider}</b> account <code>{account_id}</code> saved.</p>\
         <p>You can close this window.</p></body></html>"
    ))
    .into_response())
}

/// Upsert oauth provider + insert account with tokens. Returns account id.
pub fn save_oauth_account(
    db: &Db,
    provider_type: &str,
    tokens: &TokenSet,
    default_base_url: &str,
) -> anyhow::Result<String> {
    let mut creds = OAuthCredentials::from_token_set(tokens);
    creds.base_url = Some(default_base_url.trim_end_matches('/').to_string());
    let creds_json = serde_json::to_string(&creds)?;
    let now = Utc::now().to_rfc3339();
    let account_id = uuid::Uuid::new_v4().to_string();
    let label = format!("{provider_type}-{}", &account_id[..8]);

    db.with_conn(|conn| {
        // Reuse existing oauth provider row for this type if present.
        let provider_id: String = match conn.query_row(
            "SELECT id FROM providers WHERE kind = 'oauth' AND provider_type = ?1 LIMIT 1",
            params![provider_type],
            |r| r.get(0),
        ) {
            Ok(id) => id,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let id = uuid::Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO providers (id, kind, provider_type, name, enabled, config_json, created_at)
                     VALUES (?1, 'oauth', ?2, ?3, 1, '{}', ?4)",
                    params![
                        id,
                        provider_type,
                        format!("{} OAuth", provider_type),
                        now
                    ],
                )?;
                id
            }
            Err(e) => return Err(e),
        };

        conn.execute(
            "INSERT INTO accounts (id, provider_id, label, enabled, credentials_json, quota_json, created_at)
             VALUES (?1, ?2, ?3, 1, ?4, '{}', ?5)",
            params![account_id, provider_id, label, creds_json, now],
        )?;
        Ok(())
    })?;
    Ok(account_id)
}

/// Insert an oauth account with fully specified credentials (tests / import).
pub fn insert_oauth_account(
    db: &Db,
    provider_type: &str,
    label: &str,
    creds: &OAuthCredentials,
) -> anyhow::Result<(String, String)> {
    let creds_json = serde_json::to_string(creds)?;
    let now = Utc::now().to_rfc3339();
    let account_id = uuid::Uuid::new_v4().to_string();
    let provider_id = db.with_conn(|conn| {
        let provider_id: String = match conn.query_row(
            "SELECT id FROM providers WHERE kind = 'oauth' AND provider_type = ?1 LIMIT 1",
            params![provider_type],
            |r| r.get(0),
        ) {
            Ok(id) => id,
            Err(rusqlite::Error::QueryReturnedNoRows) => {
                let id = uuid::Uuid::new_v4().to_string();
                conn.execute(
                    "INSERT INTO providers (id, kind, provider_type, name, enabled, config_json, created_at)
                     VALUES (?1, 'oauth', ?2, ?3, 1, '{}', ?4)",
                    params![id, provider_type, format!("{provider_type} OAuth"), now],
                )?;
                id
            }
            Err(e) => return Err(e),
        };
        conn.execute(
            "INSERT INTO accounts (id, provider_id, label, enabled, credentials_json, quota_json, created_at)
             VALUES (?1, ?2, ?3, 1, ?4, '{}', ?5)",
            params![account_id, provider_id, label, creds_json, now],
        )?;
        Ok(provider_id)
    })?;
    Ok((provider_id, account_id))
}

/// Load oauth accounts into routing pools (Bearer access_token).
///
/// Merges cleanly with api_key pools via [`merge_account_pools`].
///
/// Synthetic pools (enabled only):
/// - [`OPENAI_COMPAT_POOL_KEY`]: OpenAI-shaped OAuth (`codex`, `xai`, `kimi`)
/// - [`ANTHROPIC_POOL_KEY`]: Claude OAuth (Anthropic Messages wire)
/// - `type:{provider_type}`: per-type pools for model-based routing (e.g. `type:kimi`)
///
/// Other OAuth types (e.g. `antigravity`) stay on per-`provider_id` pools only.
pub fn load_oauth_account_pools(db: &Db) -> anyhow::Result<HashMap<String, Vec<CachedAccount>>> {
    let rows = db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT a.id, a.provider_id, a.enabled, a.credentials_json,
                    p.enabled, p.provider_type
             FROM accounts a
             INNER JOIN providers p ON p.id = a.provider_id
             WHERE p.kind = 'oauth'
             ORDER BY a.created_at ASC",
        )?;
        let mapped = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(mapped)
    })?;

    let mut by_provider: HashMap<String, Vec<CachedAccount>> = HashMap::new();
    let mut openai_compat_pool: Vec<CachedAccount> = Vec::new();
    let mut anthropic_pool: Vec<CachedAccount> = Vec::new();
    let mut type_pools: HashMap<String, Vec<CachedAccount>> = HashMap::new();

    for (account_id, provider_id, acct_enabled, creds_raw, prov_enabled, provider_type) in rows {
        let creds: OAuthCredentials = match serde_json::from_str(&creds_raw) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    account_id = %account_id,
                    error = %e,
                    "skip oauth account: bad credentials_json"
                );
                continue;
            }
        };
        if creds.access_token.is_empty() {
            tracing::warn!(account_id = %account_id, "skip oauth account: empty access_token");
            continue;
        }
        let base = creds
            .base_url
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.trim_end_matches('/').to_string())
            .or_else(|| default_base_for_type(&provider_type).map(|s| s.to_string()));
        let Some(base) = base else {
            tracing::warn!(
                account_id = %account_id,
                provider_type = %provider_type,
                "skip oauth account: missing base_url"
            );
            continue;
        };
        let enabled = prov_enabled != 0 && acct_enabled != 0;
        let account = AccountRef {
            account_id: account_id.clone(),
            provider_id: provider_id.clone(),
            upstream_base: base,
            auth_header: format!("Bearer {}", creds.access_token),
            is_oauth: true,
        };
        let cached = CachedAccount {
            account: account.clone(),
            enabled,
        };
        by_provider
            .entry(provider_id)
            .or_default()
            .push(cached.clone());
        if enabled {
            if is_openai_oauth_type(&provider_type) {
                openai_compat_pool.push(cached.clone());
                type_pools
                    .entry(crate::router::type_pool_key(&provider_type))
                    .or_default()
                    .push(cached);
            } else if provider_type == "claude" {
                // Claude OAuth is Anthropic Messages-compatible only.
                anthropic_pool.push(cached);
            }
            // antigravity and unknown types: provider_id pool only.
        }
    }
    by_provider.insert(OPENAI_COMPAT_POOL_KEY.to_string(), openai_compat_pool);
    by_provider.insert(ANTHROPIC_POOL_KEY.to_string(), anthropic_pool);
    for (k, v) in type_pools {
        by_provider.insert(k, v);
    }
    Ok(by_provider)
}

/// OAuth provider_types that speak OpenAI chat/completions wire format.
fn is_openai_oauth_type(provider_type: &str) -> bool {
    matches!(provider_type, "codex" | "xai" | "kimi")
}

fn default_base_for_type(provider_type: &str) -> Option<&'static str> {
    match provider_type {
        "codex" => Some(codex::CODEX_DEFAULT_BASE_URL),
        "claude" => Some(claude::CLAUDE_DEFAULT_BASE_URL),
        "antigravity" => Some(antigravity::ANTIGRAVITY_DEFAULT_BASE_URL),
        "xai" => Some(xai::XAI_DEFAULT_BASE_URL),
        "kimi" => Some(kimi::KIMI_DEFAULT_BASE_URL),
        _ => None,
    }
}

/// Merge oauth pools into an existing (api_key) pool map.
pub fn merge_account_pools(
    mut base: HashMap<String, Vec<CachedAccount>>,
    oauth: HashMap<String, Vec<CachedAccount>>,
) -> HashMap<String, Vec<CachedAccount>> {
    for (k, mut v) in oauth {
        base.entry(k).or_default().append(&mut v);
    }
    base
}

/// Empty pending map for AppState construction.
pub fn new_pending_map() -> PendingMap {
    Arc::new(Mutex::new(HashMap::new()))
}

/// Helper used by cache reload tests.
pub fn reload_all_pools(db: &Db, cache: &ConfigCache) -> anyhow::Result<()> {
    let api = crate::providers::api_key::load_account_pools(db)?;
    let oauth = load_oauth_account_pools(db)?;
    cache.replace_account_pools(merge_account_pools(api, oauth));
    Ok(())
}

/// Redacted view of oauth credentials for admin list (if reused).
pub fn redact_oauth_credentials(creds: &OAuthCredentials) -> Value {
    let prefix: String = creds.access_token.chars().take(8).collect();
    json!({
        "access_token_prefix": if prefix.is_empty() { "***".into() } else { format!("{prefix}…") },
        "has_refresh_token": creds.refresh_token.as_ref().is_some_and(|t| !t.is_empty()),
        "expires_at": creds.expires_at,
        "base_url": creds.base_url,
    })
}
