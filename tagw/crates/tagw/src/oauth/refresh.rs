//! Token ensure + background near-expiry refresh.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use rusqlite::params;

use crate::cache::ConfigCache;
use crate::db::Db;
use crate::error::AppError;

use super::antigravity::AntigravityProvider;
use super::claude::ClaudeProvider;
use super::codex::CodexProvider;
use super::kimi::KimiProvider;
use super::types::{
    OAuthCredentials, OAuthProvider, TokenSet, ACCESS_TOKEN_REFRESH_SKEW_SECS,
    BACKGROUND_REFRESH_INTERVAL_SECS, BACKGROUND_REFRESH_LEAD_SECS,
};
use super::xai::XaiProvider;

/// Account row needed for refresh.
#[derive(Clone, Debug)]
struct AccountRow {
    id: String,
    provider_type: String,
    credentials: OAuthCredentials,
}

/// Ensure a valid access token for `account_id`.
///
/// Refreshes when `expires_at` is within [`ACCESS_TOKEN_REFRESH_SKEW_SECS`] (120s)
/// of now. On success returns the (possibly new) access token and persists
/// credentials to SQLite + reloads the config cache.
pub async fn ensure_access_token(
    db: &Db,
    cache: &ConfigCache,
    account_id: &str,
) -> Result<String, AppError> {
    ensure_access_token_with_client(db, cache, account_id, &reqwest::Client::new(), false).await
}

/// Same as [`ensure_access_token`] with an injectable HTTP client and optional force.
pub async fn ensure_access_token_with_client(
    db: &Db,
    cache: &ConfigCache,
    account_id: &str,
    http: &reqwest::Client,
    force: bool,
) -> Result<String, AppError> {
    let row = load_oauth_account(db, account_id).map_err(AppError::Internal)?;
    if !force && !row.credentials.needs_refresh(ACCESS_TOKEN_REFRESH_SKEW_SECS) {
        return Ok(row.credentials.access_token);
    }
    let refresh_token = row
        .credentials
        .refresh_token
        .as_deref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::BadRequest(format!("account {account_id} has no refresh_token")))?;

    let provider = build_provider(&row.provider_type, &row.credentials, http.clone())
        .map_err(AppError::Internal)?;
    let tokens = provider
        .refresh(refresh_token)
        .await
        .map_err(|e| AppError::Upstream(format!("oauth refresh failed: {e}")))?;

    persist_refreshed(db, &row, &tokens).map_err(AppError::Internal)?;
    if let Err(e) = cache.reload(db) {
        tracing::warn!(error = %e, account_id = %account_id, "cache reload after oauth refresh failed");
    }
    Ok(tokens.access_token)
}

fn persist_refreshed(db: &Db, row: &AccountRow, tokens: &TokenSet) -> anyhow::Result<()> {
    let mut creds = row.credentials.clone();
    creds.apply_token_set(tokens);
    let json = serde_json::to_string(&creds).context("serialize oauth credentials")?;
    db.with_conn(|conn| {
        conn.execute(
            "UPDATE accounts SET credentials_json = ?1 WHERE id = ?2",
            params![json, row.id],
        )?;
        Ok(())
    })
    .context("update oauth credentials_json")?;
    Ok(())
}

fn load_oauth_account(db: &Db, account_id: &str) -> anyhow::Result<AccountRow> {
    let row = db.with_conn(|conn| {
        conn.query_row(
            "SELECT a.id, a.enabled, a.credentials_json, p.provider_type, p.enabled
             FROM accounts a
             INNER JOIN providers p ON p.id = a.provider_id
             WHERE a.id = ?1 AND p.kind = 'oauth'",
            params![account_id],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            },
        )
    });
    let (id, _enabled, creds_raw, provider_type, _provider_enabled) = match row {
        Ok(v) => v,
        Err(e) => {
            if let Some(rusqlite::Error::QueryReturnedNoRows) = e.downcast_ref::<rusqlite::Error>()
            {
                return Err(anyhow!("oauth account {account_id} not found"));
            }
            return Err(e).context("load oauth account");
        }
    };
    let credentials: OAuthCredentials =
        serde_json::from_str(&creds_raw).context("parse oauth credentials_json")?;
    Ok(AccountRow {
        id,
        provider_type,
        credentials,
    })
}

/// Build a concrete provider, applying credential-level endpoint overrides (tests).
pub fn build_provider(
    provider_type: &str,
    creds: &OAuthCredentials,
    http: reqwest::Client,
) -> anyhow::Result<Box<dyn OAuthProvider>> {
    let token_override = creds.token_url.clone();
    let client_override = creds.client_id.clone();
    match provider_type {
        "codex" => {
            let mut p = CodexProvider::new(http);
            if let Some(t) = token_override {
                p = p.with_endpoints(t, None);
            }
            if let Some(c) = client_override {
                p = p.with_client_id(c);
            }
            Ok(Box::new(p))
        }
        "claude" => {
            let mut p = ClaudeProvider::new(http);
            if let Some(t) = token_override {
                p = p.with_endpoints(t, None);
            }
            if let Some(c) = client_override {
                p.client_id = c;
            }
            Ok(Box::new(p))
        }
        "antigravity" => {
            let mut p = AntigravityProvider::new(http);
            if let Some(t) = token_override {
                p = p.with_endpoints(t, None);
            }
            if let Some(c) = client_override {
                p.client_id = c;
            }
            if let Some(s) = creds.client_secret.clone() {
                p.client_secret = s;
            }
            Ok(Box::new(p))
        }
        "xai" => {
            let mut p = XaiProvider::new(http);
            if let Some(t) = token_override {
                p = p.with_endpoints(t, None);
            }
            if let Some(c) = client_override {
                p.client_id = c;
            }
            Ok(Box::new(p))
        }
        "kimi" => {
            let mut p = KimiProvider::new(http);
            if let Some(t) = token_override {
                p = p.with_endpoints(t, None);
            }
            if let Some(c) = client_override {
                p.client_id = c;
            }
            Ok(Box::new(p))
        }
        other => Err(anyhow!("unsupported oauth provider_type '{other}'")),
    }
}

/// Public helper: build default provider by id (routes).
pub fn provider_by_id(id: &str, http: reqwest::Client) -> Option<Box<dyn OAuthProvider>> {
    match id {
        "codex" => Some(Box::new(CodexProvider::new(http))),
        "claude" => Some(Box::new(ClaudeProvider::new(http))),
        "antigravity" => Some(Box::new(AntigravityProvider::new(http))),
        "xai" => Some(Box::new(XaiProvider::new(http))),
        "kimi" => Some(Box::new(KimiProvider::new(http))),
        _ => None,
    }
}

/// List oauth account ids whose access token expires before `before`.
fn list_near_expiry(db: &Db, before: DateTime<Utc>) -> anyhow::Result<Vec<String>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT a.id, a.credentials_json
             FROM accounts a
             INNER JOIN providers p ON p.id = a.provider_id
             WHERE p.kind = 'oauth' AND a.enabled = 1 AND p.enabled = 1",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, creds_raw) = row?;
            let Ok(creds) = serde_json::from_str::<OAuthCredentials>(&creds_raw) else {
                continue;
            };
            match creds.expires_at {
                Some(exp) if exp <= before => {
                    if creds.refresh_token.as_ref().is_some_and(|t| !t.is_empty()) {
                        out.push(id);
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    })
}

/// One background pass: refresh all accounts expiring within the lead window.
pub async fn refresh_near_expiry_accounts(db: &Db, cache: &ConfigCache, http: &reqwest::Client) {
    let before = Utc::now() + chrono::Duration::seconds(BACKGROUND_REFRESH_LEAD_SECS);
    let ids = match list_near_expiry(db, before) {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(error = %e, "oauth background: list near-expiry failed");
            return;
        }
    };
    for id in ids {
        match ensure_access_token_with_client(db, cache, &id, http, true).await {
            Ok(_) => {
                tracing::info!(account_id = %id, "oauth background refresh ok");
            }
            Err(e) => {
                tracing::warn!(account_id = %id, error = %e, "oauth background refresh failed");
            }
        }
    }
}

/// Spawn a 60s loop that refreshes near-expiry oauth accounts.
/// Returns a JoinHandle so the caller can keep it alive.
pub fn spawn_oauth_refresh_loop(
    db: Db,
    cache: ConfigCache,
    http: reqwest::Client,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(Duration::from_secs(BACKGROUND_REFRESH_INTERVAL_SECS));
        // Skip the immediate first tick burst — wait one full interval.
        interval.tick().await;
        loop {
            interval.tick().await;
            refresh_near_expiry_accounts(&db, &cache, &http).await;
        }
    })
}

/// Shared pending-state map type alias for routes.
pub type PendingMap = Arc<std::sync::Mutex<std::collections::HashMap<String, super::types::PendingOAuth>>>;
