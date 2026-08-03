//! Optional Keycloak OIDC login for the dashboard.
//!
//! Settings (`settings` table keys):
//! - `oidc.enabled` — JSON bool (default false)
//! - `oidc.issuer` — Keycloak realm issuer URL
//! - `oidc.client_id`
//! - `oidc.client_secret`
//! - `oidc.redirect_uri` — must match IdP client config
//!
//! Endpoints (issuer-based Keycloak paths, no discovery document required):
//! - authorize: `{issuer}/protocol/openid-connect/auth`
//! - token:     `{issuer}/protocol/openid-connect/token`
//! - userinfo:  `{issuer}/protocol/openid-connect/userinfo`
//!
//! On success: create/link `users` by `oidc_sub`, mint the same `tagw_session`
//! cookie as basic auth. Role defaults to `viewer` unless
//! `realm_access.roles` contains `tagw-admin`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::header::SET_COOKIE;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use axum::{Json, Router};
use base64::engine::general_purpose::{URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use chrono::Utc;
use rand_core::{OsRng, RngCore};
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use serde_json::{json, Value};
use url::Url;

use crate::auth::dashboard::{
    mint_session_token, session_set_cookie, DashboardUser, Role, Session,
};
use crate::db::Db;
use crate::error::AppError;
use crate::state::AppState;

/// Role claim that grants dashboard admin (Keycloak realm role).
pub const TAGW_ADMIN_ROLE: &str = "tagw-admin";

/// Max age for in-memory OIDC start sessions (CSRF state).
const PENDING_TTL: Duration = Duration::from_secs(10 * 60);

// ── Settings keys ───────────────────────────────────────────────────────────

pub const SETTING_ENABLED: &str = "oidc.enabled";
pub const SETTING_ISSUER: &str = "oidc.issuer";
pub const SETTING_CLIENT_ID: &str = "oidc.client_id";
pub const SETTING_CLIENT_SECRET: &str = "oidc.client_secret";
pub const SETTING_REDIRECT_URI: &str = "oidc.redirect_uri";

// ── Settings helpers ────────────────────────────────────────────────────────

/// Read a setting value_json as serde_json::Value.
pub fn get_setting(db: &Db, key: &str) -> Result<Option<Value>, AppError> {
    db.with_conn(|conn| {
        conn.query_row(
            "SELECT value_json FROM settings WHERE key = ?1",
            params![key],
            |r| r.get::<_, String>(0),
        )
        .optional()
    })
    .map_err(AppError::Internal)?
    .map(|raw| {
        serde_json::from_str(&raw)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("settings '{key}' invalid json: {e}")))
    })
    .transpose()
}

/// Upsert a setting (value stored as JSON text).
pub fn set_setting(db: &Db, key: &str, value: &Value) -> Result<(), AppError> {
    let raw = serde_json::to_string(value)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize setting: {e}")))?;
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO settings (key, value_json) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
            params![key, raw],
        )
    })
    .map_err(AppError::Internal)?;
    Ok(())
}

fn setting_bool(db: &Db, key: &str, default: bool) -> Result<bool, AppError> {
    Ok(match get_setting(db, key)? {
        Some(Value::Bool(b)) => b,
        Some(Value::String(s)) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => true,
            "false" | "0" | "no" => false,
            _ => default,
        },
        Some(Value::Number(n)) => n.as_i64().map(|i| i != 0).unwrap_or(default),
        _ => default,
    })
}

fn setting_string(db: &Db, key: &str) -> Result<Option<String>, AppError> {
    Ok(match get_setting(db, key)? {
        Some(Value::String(s)) if !s.trim().is_empty() => Some(s),
        Some(Value::String(_)) => None,
        Some(other) => {
            // Allow non-string JSON by stringifying primitives carefully.
            match other {
                Value::Null => None,
                v => {
                    let s = v.as_str().map(|s| s.to_string()).unwrap_or_else(|| {
                        // Strip quotes from to_string for pure strings already handled.
                        v.to_string().trim_matches('"').to_string()
                    });
                    if s.is_empty() || s == "null" {
                        None
                    } else {
                        Some(s)
                    }
                }
            }
        }
        None => None,
    })
}

/// OIDC client configuration loaded from the settings table.
#[derive(Clone, Debug)]
pub struct OidcConfig {
    pub enabled: bool,
    pub issuer: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

impl OidcConfig {
    pub fn is_usable(&self) -> bool {
        self.enabled
            && !self.issuer.is_empty()
            && !self.client_id.is_empty()
            && !self.redirect_uri.is_empty()
    }

    fn issuer_base(&self) -> String {
        self.issuer.trim_end_matches('/').to_string()
    }

    pub fn authorize_url(&self) -> String {
        format!(
            "{}/protocol/openid-connect/auth",
            self.issuer_base()
        )
    }

    pub fn token_url(&self) -> String {
        format!(
            "{}/protocol/openid-connect/token",
            self.issuer_base()
        )
    }

    pub fn userinfo_url(&self) -> String {
        format!(
            "{}/protocol/openid-connect/userinfo",
            self.issuer_base()
        )
    }
}

/// Load OIDC settings from SQLite.
pub fn load_oidc_config(db: &Db) -> Result<OidcConfig, AppError> {
    Ok(OidcConfig {
        enabled: setting_bool(db, SETTING_ENABLED, false)?,
        issuer: setting_string(db, SETTING_ISSUER)?.unwrap_or_default(),
        client_id: setting_string(db, SETTING_CLIENT_ID)?.unwrap_or_default(),
        client_secret: setting_string(db, SETTING_CLIENT_SECRET)?.unwrap_or_default(),
        redirect_uri: setting_string(db, SETTING_REDIRECT_URI)?.unwrap_or_default(),
    })
}

/// Convenience for tests / admin: write full OIDC config.
pub fn save_oidc_config(db: &Db, cfg: &OidcConfig) -> Result<(), AppError> {
    set_setting(db, SETTING_ENABLED, &json!(cfg.enabled))?;
    set_setting(db, SETTING_ISSUER, &json!(cfg.issuer))?;
    set_setting(db, SETTING_CLIENT_ID, &json!(cfg.client_id))?;
    set_setting(db, SETTING_CLIENT_SECRET, &json!(cfg.client_secret))?;
    set_setting(db, SETTING_REDIRECT_URI, &json!(cfg.redirect_uri))?;
    Ok(())
}

// ── Pending CSRF state ──────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct PendingOidc {
    created_at: chrono::DateTime<Utc>,
    redirect_uri: String,
}

fn pending_map() -> &'static Mutex<HashMap<String, PendingOidc>> {
    static PENDING: OnceLock<Mutex<HashMap<String, PendingOidc>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn purge_stale(map: &mut HashMap<String, PendingOidc>) {
    let cutoff = Utc::now() - chrono::Duration::from_std(PENDING_TTL).unwrap_or_default();
    map.retain(|_, p| p.created_at > cutoff);
}

fn random_state() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

// ── Claims / role ───────────────────────────────────────────────────────────

/// Map Keycloak claims → dashboard role.
/// Default `viewer`; `admin` if `realm_access.roles` contains `tagw-admin`.
pub fn role_from_claims(claims: &Value) -> Role {
    if let Some(roles) = claims
        .pointer("/realm_access/roles")
        .and_then(|v| v.as_array())
    {
        if roles
            .iter()
            .any(|r| r.as_str() == Some(TAGW_ADMIN_ROLE))
        {
            return Role::Admin;
        }
    }
    Role::Viewer
}

/// Preferred username for a new OIDC user.
pub fn username_from_claims(claims: &Value, sub: &str) -> String {
    for key in ["preferred_username", "email", "name"] {
        if let Some(s) = claims.get(key).and_then(|v| v.as_str()) {
            let t = s.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    let short = if sub.len() > 12 { &sub[..12] } else { sub };
    format!("oidc-{short}")
}

/// Decode JWT payload without signature verification.
///
/// Safe here because the token is obtained directly from the token endpoint
/// we just authenticated with (confidential client), not from a browser-supplied JWT.
pub fn decode_jwt_payload(token: &str) -> Result<Value, AppError> {
    let mut parts = token.split('.');
    let _header = parts
        .next()
        .ok_or_else(|| AppError::Upstream("malformed id_token".into()))?;
    let payload_b64 = parts
        .next()
        .ok_or_else(|| AppError::Upstream("malformed id_token (no payload)".into()))?;
    let bytes = URL_SAFE_NO_PAD
        .decode(payload_b64)
        .or_else(|_| URL_SAFE.decode(payload_b64))
        .map_err(|e| AppError::Upstream(format!("id_token payload b64: {e}")))?;
    serde_json::from_slice(&bytes)
        .map_err(|e| AppError::Upstream(format!("id_token payload json: {e}")))
}

// ── User create / link ──────────────────────────────────────────────────────

/// Find user by `oidc_sub`.
pub fn load_user_by_oidc_sub(db: &Db, sub: &str) -> Result<Option<DashboardUser>, AppError> {
    db.with_conn(|conn| {
        conn.query_row(
            "SELECT id, username, role FROM users WHERE oidc_sub = ?1",
            params![sub],
            |r| {
                let role_s: String = r.get(2)?;
                Ok(DashboardUser {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    role: Role::parse(&role_s).unwrap_or(Role::Viewer),
                })
            },
        )
        .optional()
    })
    .map_err(AppError::Internal)
}

fn username_taken(db: &Db, username: &str) -> Result<bool, AppError> {
    let n: i64 = db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT COUNT(*) FROM users WHERE username = ?1",
                params![username],
                |r| r.get(0),
            )
        })
        .map_err(AppError::Internal)?;
    Ok(n > 0)
}

/// Create or link a dashboard user for an OIDC subject. Updates role on each login.
pub fn upsert_oidc_user(
    db: &Db,
    sub: &str,
    preferred_username: &str,
    role: Role,
) -> Result<DashboardUser, AppError> {
    if sub.is_empty() {
        return Err(AppError::Upstream("OIDC sub is empty".into()));
    }

    if let Some(existing) = load_user_by_oidc_sub(db, sub)? {
        // Keep role in sync with IdP claims.
        if existing.role != role {
            db.with_conn(|conn| {
                conn.execute(
                    "UPDATE users SET role = ?1 WHERE id = ?2",
                    params![role.as_str(), existing.id],
                )
            })
            .map_err(AppError::Internal)?;
            return Ok(DashboardUser {
                id: existing.id,
                username: existing.username,
                role,
            });
        }
        return Ok(existing);
    }

    // Resolve a unique username.
    let mut username = preferred_username.trim().to_string();
    if username.is_empty() {
        username = username_from_claims(&json!({}), sub);
    }
    if username_taken(db, &username)? {
        let short = if sub.len() > 8 { &sub[..8] } else { sub };
        username = format!("{username}-{short}");
        // Extremely unlikely still taken; fall back to full sub.
        if username_taken(db, &username)? {
            username = format!("oidc-{sub}");
        }
    }

    let id = uuid::Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO users (id, username, password_hash, oidc_sub, role, created_at)
             VALUES (?1, ?2, NULL, ?3, ?4, ?5)",
            params![id, username, sub, role.as_str(), created_at],
        )
    })
    .map_err(AppError::Internal)?;

    Ok(DashboardUser {
        id,
        username,
        role,
    })
}

// ── Token exchange ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: Option<String>,
    id_token: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

/// Exchange authorization code at Keycloak token endpoint.
pub async fn exchange_code(
    http: &reqwest::Client,
    cfg: &OidcConfig,
    code: &str,
    redirect_uri: &str,
) -> Result<(String, Option<Value>), AppError> {
    let body = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
    ];

    let res = http
        .post(cfg.token_url())
        .header("content-type", "application/x-www-form-urlencoded")
        .form(&body)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("oidc token request: {e}")))?;

    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("oidc token body: {e}")))?;

    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "oidc token endpoint {status}: {text}"
        )));
    }

    let tok: TokenResponse = serde_json::from_str(&text)
        .map_err(|e| AppError::Upstream(format!("oidc token json: {e}")))?;

    if let Some(err) = tok.error {
        let desc = tok.error_description.unwrap_or_default();
        return Err(AppError::Upstream(format!("oidc token error: {err} {desc}")));
    }

    let access_token = tok
        .access_token
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::Upstream("oidc token response missing access_token".into()))?;

    let id_claims = match tok.id_token.as_deref() {
        Some(id) if !id.is_empty() => Some(decode_jwt_payload(id)?),
        _ => None,
    };

    Ok((access_token, id_claims))
}

/// Fetch userinfo when id_token claims are insufficient / missing.
pub async fn fetch_userinfo(
    http: &reqwest::Client,
    cfg: &OidcConfig,
    access_token: &str,
) -> Result<Value, AppError> {
    let res = http
        .get(cfg.userinfo_url())
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("oidc userinfo request: {e}")))?;
    let status = res.status();
    let text = res
        .text()
        .await
        .map_err(|e| AppError::Upstream(format!("oidc userinfo body: {e}")))?;
    if !status.is_success() {
        return Err(AppError::Upstream(format!(
            "oidc userinfo {status}: {text}"
        )));
    }
    serde_json::from_str(&text)
        .map_err(|e| AppError::Upstream(format!("oidc userinfo json: {e}")))
}

/// Complete OIDC login given code + redirect_uri used at start (testable core).
pub async fn complete_oidc_login(
    db: &Db,
    http: &reqwest::Client,
    session_secret: &str,
    cfg: &OidcConfig,
    code: &str,
    redirect_uri: &str,
) -> Result<Session, AppError> {
    let (access_token, id_claims) = exchange_code(http, cfg, code, redirect_uri).await?;

    let claims = if let Some(c) = id_claims {
        // Prefer id_token; fill sub from userinfo if missing.
        if c.get("sub").and_then(|v| v.as_str()).is_some() {
            c
        } else {
            fetch_userinfo(http, cfg, &access_token).await?
        }
    } else {
        fetch_userinfo(http, cfg, &access_token).await?
    };

    let sub = claims
        .get("sub")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::Upstream("OIDC claims missing sub".into()))?
        .to_string();

    let role = role_from_claims(&claims);
    let preferred = username_from_claims(&claims, &sub);
    let user = upsert_oidc_user(db, &sub, &preferred, role)?;
    let token = mint_session_token(&user.id, session_secret);
    Ok(Session { user, token })
}

// ── HTTP routes ─────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/oidc/start", get(oidc_start))
        .route("/api/auth/oidc/callback", get(oidc_callback))
}

#[derive(Debug, Deserialize)]
struct StartQuery {
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
    /// When true, return JSON user instead of redirect (tests / SPA).
    #[serde(default)]
    json: bool,
}

async fn oidc_start(
    State(state): State<AppState>,
    Query(q): Query<StartQuery>,
) -> Result<Response, AppError> {
    let cfg = load_oidc_config(&state.db)?;
    if !cfg.enabled {
        return Err(AppError::BadRequest("OIDC is disabled".into()));
    }
    if !cfg.is_usable() {
        return Err(AppError::BadRequest(
            "OIDC is enabled but incomplete (issuer, client_id, redirect_uri required)".into(),
        ));
    }

    let state_param = random_state();
    {
        let mut guard = pending_map().lock().expect("oidc pending lock");
        purge_stale(&mut guard);
        guard.insert(
            state_param.clone(),
            PendingOidc {
                created_at: Utc::now(),
                redirect_uri: cfg.redirect_uri.clone(),
            },
        );
    }

    let mut url = Url::parse(&cfg.authorize_url())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("bad oidc authorize url: {e}")))?;
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("response_type", "code");
        pairs.append_pair("client_id", &cfg.client_id);
        pairs.append_pair("redirect_uri", &cfg.redirect_uri);
        pairs.append_pair("scope", "openid profile email");
        pairs.append_pair("state", &state_param);
    }
    let authorize_url = url.to_string();

    if q.redirect {
        Ok(Redirect::temporary(&authorize_url).into_response())
    } else {
        Ok(Json(json!({
            "authorize_url": authorize_url,
            "state": state_param,
            "redirect_uri": cfg.redirect_uri,
        }))
        .into_response())
    }
}

async fn oidc_callback(
    State(state): State<AppState>,
    Query(q): Query<CallbackQuery>,
) -> Result<Response, AppError> {
    let cfg = load_oidc_config(&state.db)?;
    if !cfg.enabled {
        return Err(AppError::BadRequest("OIDC is disabled".into()));
    }

    if let Some(err) = q.error {
        let desc = q.error_description.unwrap_or_default();
        return Err(AppError::BadRequest(format!("OIDC error: {err} {desc}")));
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
        let mut guard = pending_map().lock().expect("oidc pending lock");
        purge_stale(&mut guard);
        guard.remove(&state_param)
    }
    .ok_or_else(|| AppError::BadRequest("unknown or expired OIDC state".into()))?;

    let session = complete_oidc_login(
        &state.db,
        &state.http_client,
        &state.session_secret,
        &cfg,
        &code,
        &pending.redirect_uri,
    )
    .await?;

    if q.json {
        let mut res = (StatusCode::OK, Json(session.user)).into_response();
        if let Ok(val) = HeaderValue::from_str(&session_set_cookie(&session.token)) {
            res.headers_mut().insert(SET_COOKIE, val);
        }
        return Ok(res);
    }

    // Browser: set session cookie and land on dashboard root.
    let mut res = Redirect::temporary("/").into_response();
    if let Ok(val) = HeaderValue::from_str(&session_set_cookie(&session.token)) {
        res.headers_mut().insert(SET_COOKIE, val);
    }
    Ok(res)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_default_viewer() {
        let claims = json!({"sub": "u1", "realm_access": {"roles": ["offline_access"]}});
        assert_eq!(role_from_claims(&claims), Role::Viewer);
    }

    #[test]
    fn role_tagw_admin() {
        let claims = json!({
            "sub": "u1",
            "realm_access": {"roles": ["default-roles-tagw", "tagw-admin"]}
        });
        assert_eq!(role_from_claims(&claims), Role::Admin);
    }

    #[test]
    fn jwt_payload_roundtrip() {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"none"}"#);
        let payload = URL_SAFE_NO_PAD.encode(
            br#"{"sub":"abc","preferred_username":"alice","realm_access":{"roles":["tagw-admin"]}}"#,
        );
        let token = format!("{header}.{payload}.x");
        let claims = decode_jwt_payload(&token).unwrap();
        assert_eq!(claims["sub"], "abc");
        assert_eq!(role_from_claims(&claims), Role::Admin);
        assert_eq!(username_from_claims(&claims, "abc"), "alice");
    }
}
