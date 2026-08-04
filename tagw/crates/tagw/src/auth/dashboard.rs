//! Dashboard basic auth + signed session cookie + RBAC extractors.
//!
//! Session cookie: `tagw_session` = `{user_id}.{exp_unix}.{hmac_hex}`
//! HMAC-SHA256 over `{user_id}.{exp_unix}` with `TAGW_SESSION_SECRET`.

use axum::extract::FromRequestParts;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use hmac::{Hmac, Mac};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

use crate::db::Db;
use crate::error::AppError;
use crate::state::AppState;

/// Cookie name for the signed dashboard session.
pub const SESSION_COOKIE: &str = "tagw_session";

/// Insecure default used only when `TAGW_SESSION_SECRET` is unset (dev).
pub const DEFAULT_SESSION_SECRET: &str = "tagw-dev-session-secret-change-me";

/// Session lifetime (7 days).
pub const SESSION_MAX_AGE_SECS: i64 = 7 * 24 * 60 * 60;

type HmacSha256 = Hmac<Sha256>;

/// Dashboard RBAC role (matches `users.role` CHECK).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Viewer,
    Admin,
}

impl Role {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "viewer" => Some(Self::Viewer),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Admin => "admin",
        }
    }

    /// Whether this role satisfies the minimum required role.
    pub fn meets(self, min: Role) -> bool {
        match min {
            Role::Viewer => true,
            Role::Admin => matches!(self, Role::Admin),
        }
    }
}

/// Authenticated dashboard identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct DashboardUser {
    pub id: String,
    pub username: String,
    pub role: Role,
}

/// Issued session after successful login.
#[derive(Clone, Debug)]
pub struct Session {
    pub user: DashboardUser,
    /// Raw cookie value (signed token).
    pub token: String,
}

/// Resolve session secret from env; warn and fall back to dev default.
pub fn resolve_session_secret() -> String {
    match std::env::var("TAGW_SESSION_SECRET") {
        Ok(s) if !s.is_empty() => s,
        Ok(_) => {
            tracing::warn!(
                "TAGW_SESSION_SECRET is empty — using insecure dev default; set a strong secret in production"
            );
            DEFAULT_SESSION_SECRET.to_string()
        }
        Err(_) => {
            tracing::warn!(
                "TAGW_SESSION_SECRET not set — using insecure dev default; set a strong secret in production"
            );
            DEFAULT_SESSION_SECRET.to_string()
        }
    }
}

/// Require `user.role` to meet `min` (viewer < admin).
pub fn require_role(user: &DashboardUser, min: Role) -> Result<(), AppError> {
    if user.role.meets(min) {
        Ok(())
    } else {
        Err(AppError::Forbidden)
    }
}

// ── Password / users ────────────────────────────────────────────────────────

fn verify_password(password: &str, hash: &str) -> bool {
    use argon2::password_hash::{PasswordHash, PasswordVerifier};
    use argon2::Argon2;
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Hash a password with argon2 (PHC string). Used for user create/seed.
pub fn hash_password(password: &str) -> Result<String, AppError> {
    use argon2::password_hash::{PasswordHasher, SaltString};
    use argon2::Argon2;
    use rand_core::OsRng;
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| AppError::Internal(anyhow::anyhow!("argon2 hash failed: {e}")))
}

/// Load a user by primary key.
pub fn load_user_by_id(db: &Db, id: &str) -> Result<Option<DashboardUser>, AppError> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, username, role FROM users WHERE id = ?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(r) = rows.next()? {
            let role_s: String = r.get(2)?;
            let role = Role::parse(&role_s).unwrap_or(Role::Viewer);
            Ok(Some(DashboardUser {
                id: r.get(0)?,
                username: r.get(1)?,
                role,
            }))
        } else {
            Ok(None)
        }
    })
    .map_err(AppError::Internal)
}

/// Load a user by username (for login / tests).
pub fn load_user_by_username(db: &Db, username: &str) -> Result<Option<DashboardUser>, AppError> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, username, role FROM users WHERE username = ?1",
        )?;
        let mut rows = stmt.query(params![username])?;
        if let Some(r) = rows.next()? {
            let role_s: String = r.get(2)?;
            let role = Role::parse(&role_s).unwrap_or(Role::Viewer);
            Ok(Some(DashboardUser {
                id: r.get(0)?,
                username: r.get(1)?,
                role,
            }))
        } else {
            Ok(None)
        }
    })
    .map_err(AppError::Internal)
}

/// Basic username/password login → signed session.
pub fn login_basic(db: &Db, username: &str, password: &str, secret: &str) -> Result<Session, AppError> {
    let username = username.trim();
    if username.is_empty() || password.is_empty() {
        return Err(AppError::Unauthorized);
    }

    let row: Option<(String, String, String, Option<String>)> = db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT id, username, role, password_hash FROM users WHERE username = ?1",
                params![username],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()
        })
        .map_err(AppError::Internal)?;

    let Some((id, uname, role_s, password_hash)) = row else {
        return Err(AppError::Unauthorized);
    };
    let Some(hash) = password_hash.filter(|h| !h.is_empty()) else {
        // OIDC-only users have no password.
        return Err(AppError::Unauthorized);
    };
    if !verify_password(password, &hash) {
        return Err(AppError::Unauthorized);
    }
    let role = Role::parse(&role_s).unwrap_or(Role::Viewer);
    let user = DashboardUser {
        id: id.clone(),
        username: uname,
        role,
    };
    let token = mint_session_token(&id, secret);
    Ok(Session { user, token })
}

// ── Signed session cookie ───────────────────────────────────────────────────

/// Mint a signed session token for `user_id` (also used by integration tests).
pub fn mint_session_token(user_id: &str, secret: &str) -> String {
    let exp = chrono::Utc::now().timestamp() + SESSION_MAX_AGE_SECS;
    mint_session_token_exp(user_id, exp, secret)
}

fn mint_session_token_exp(user_id: &str, exp: i64, secret: &str) -> String {
    let payload = format!("{user_id}.{exp}");
    let sig = sign(secret, &payload);
    format!("{payload}.{sig}")
}

fn sign(secret: &str, payload: &str) -> String {
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key length");
    mac.update(payload.as_bytes());
    hex_encode(&mac.finalize().into_bytes())
}

fn verify_sig(secret: &str, payload: &str, sig_hex: &str) -> bool {
    let expected = sign(secret, payload);
    // Constant-time compare via hmac verify would need raw bytes; equal length hex compare.
    if expected.len() != sig_hex.len() {
        return false;
    }
    expected
        .bytes()
        .zip(sig_hex.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// Parse and verify a session cookie value → user id (if valid and not expired).
pub fn verify_session_token(token: &str, secret: &str) -> Option<String> {
    let mut parts = token.splitn(3, '.');
    let user_id = parts.next()?;
    let exp_s = parts.next()?;
    let sig = parts.next()?;
    if user_id.is_empty() || sig.is_empty() {
        return None;
    }
    let exp: i64 = exp_s.parse().ok()?;
    if exp < chrono::Utc::now().timestamp() {
        return None;
    }
    let payload = format!("{user_id}.{exp_s}");
    if !verify_sig(secret, &payload, sig) {
        return None;
    }
    Some(user_id.to_string())
}

/// Build `Set-Cookie` value for a session token.
pub fn session_set_cookie(token: &str) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_MAX_AGE_SECS}"
    )
}

/// Clear session cookie.
pub fn session_clear_cookie() -> String {
    format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
}

fn cookie_from_headers(headers: &HeaderMap, name: &str) -> Option<String> {
    let cookie = headers.get(COOKIE)?.to_str().ok()?;
    for part in cookie.split(';') {
        let part = part.trim();
        if let Some((k, v)) = part.split_once('=') {
            if k.trim() == name {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Resolve dashboard user from request cookies + DB.
pub fn user_from_request(
    headers: &HeaderMap,
    db: &Db,
    secret: &str,
) -> Result<DashboardUser, AppError> {
    let token =
        cookie_from_headers(headers, SESSION_COOKIE).ok_or(AppError::Unauthorized)?;
    let user_id = verify_session_token(&token, secret).ok_or(AppError::Unauthorized)?;
    let user = load_user_by_id(db, &user_id)?.ok_or(AppError::Unauthorized)?;
    Ok(user)
}

// ── Axum extractors ─────────────────────────────────────────────────────────

/// Any authenticated dashboard user (viewer or admin).
#[derive(Clone, Debug)]
pub struct AuthUser(pub DashboardUser);

/// Admin-only dashboard user.
#[derive(Clone, Debug)]
pub struct AdminUser(pub DashboardUser);

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = user_from_request(&parts.headers, &state.db, &state.session_secret)?;
        Ok(AuthUser(user))
    }
}

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = user_from_request(&parts.headers, &state.db, &state.session_secret)?;
        require_role(&user, Role::Admin)?;
        Ok(AdminUser(user))
    }
}

// ── HTTP routes ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Auth routes (login is open; me requires session).
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/auth/login", post(login_handler))
        .route("/api/auth/logout", post(logout_handler))
        .route("/api/auth/me", get(me_handler))
}

async fn login_handler(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<Response, AppError> {
    let session = login_basic(
        &state.db,
        &body.username,
        &body.password,
        &state.session_secret,
    )?;
    let mut res = (StatusCode::OK, Json(session.user)).into_response();
    if let Ok(val) = HeaderValue::from_str(&session_set_cookie(&session.token)) {
        res.headers_mut().insert(SET_COOKIE, val);
    }
    Ok(res)
}

async fn logout_handler() -> Response {
    let mut res = (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response();
    if let Ok(val) = HeaderValue::from_str(&session_clear_cookie()) {
        res.headers_mut().insert(SET_COOKIE, val);
    }
    res
}

async fn me_handler(AuthUser(user): AuthUser) -> Json<DashboardUser> {
    Json(user)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_meets_admin_requires_admin() {
        assert!(Role::Admin.meets(Role::Admin));
        assert!(Role::Admin.meets(Role::Viewer));
        assert!(!Role::Viewer.meets(Role::Admin));
        assert!(Role::Viewer.meets(Role::Viewer));
    }

    #[test]
    fn session_token_roundtrip() {
        let secret = "test-secret";
        let token = mint_session_token("user-123", secret);
        let uid = verify_session_token(&token, secret).expect("valid");
        assert_eq!(uid, "user-123");
        assert!(verify_session_token(&token, "wrong").is_none());
        // Tamper
        let mut bad = token.clone();
        bad.push('x');
        assert!(verify_session_token(&bad, secret).is_none());
    }

    #[test]
    fn expired_token_rejected() {
        let secret = "test-secret";
        let past = chrono::Utc::now().timestamp() - 10;
        let token = mint_session_token_exp("u1", past, secret);
        assert!(verify_session_token(&token, secret).is_none());
    }
}
