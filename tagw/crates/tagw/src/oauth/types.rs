//! Shared OAuth types: tokens, PKCE, credentials, provider trait.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Seconds before expiry when `ensure_access_token` triggers a refresh.
pub const ACCESS_TOKEN_REFRESH_SKEW_SECS: i64 = 120;

/// Background scanner: refresh accounts expiring within this window.
pub const BACKGROUND_REFRESH_LEAD_SECS: i64 = 5 * 60;

/// Background scanner interval.
pub const BACKGROUND_REFRESH_INTERVAL_SECS: u64 = 60;

/// PKCE material for authorization-code exchange.
#[derive(Clone, Debug)]
pub struct Pkce {
    pub code_verifier: String,
    pub code_challenge: String,
    pub state: String,
    pub redirect_uri: String,
}

/// Tokens returned by exchange or refresh.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

/// Stored in `accounts.credentials_json` for `kind = oauth` providers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OAuthCredentials {
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// RFC3339 expiry of the access token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    /// Upstream OpenAI-compatible origin for the proxy pool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Optional token endpoint override (tests / self-hosted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_url: Option<String>,
    /// Optional client_id override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    /// Optional client_secret (Google / Antigravity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_secret: Option<String>,
    /// Provider-specific extras (project_id, device_id, email, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra: Option<serde_json::Value>,
}

impl OAuthCredentials {
    pub fn from_token_set(tokens: &TokenSet) -> Self {
        Self {
            access_token: tokens.access_token.clone(),
            refresh_token: tokens.refresh_token.clone(),
            expires_at: tokens.expires_at,
            base_url: None,
            token_url: None,
            client_id: None,
            client_secret: None,
            extra: None,
        }
    }

    /// Merge a refreshed token set, preserving overrides and extras.
    pub fn apply_token_set(&mut self, tokens: &TokenSet) {
        self.access_token = tokens.access_token.clone();
        if let Some(ref rt) = tokens.refresh_token {
            self.refresh_token = Some(rt.clone());
        }
        self.expires_at = tokens.expires_at;
    }

    /// True when access token is missing expiry or expires within `skew_secs`.
    pub fn needs_refresh(&self, skew_secs: i64) -> bool {
        match self.expires_at {
            None => false,
            Some(exp) => {
                let deadline = Utc::now() + chrono::Duration::seconds(skew_secs);
                exp <= deadline
            }
        }
    }
}

/// Provider-specific OAuth implementation.
#[async_trait]
pub trait OAuthProvider: Send + Sync {
    fn id(&self) -> &'static str;

    /// Build the browser authorize URL for a start request.
    fn authorize_url(&self, pkce: &Pkce) -> String;

    async fn exchange_code(&self, code: &str, pkce: &Pkce) -> anyhow::Result<TokenSet>;

    async fn refresh(&self, refresh_token: &str) -> anyhow::Result<TokenSet>;

    /// Default upstream base for account pools (OpenAI-compatible origin).
    fn default_base_url(&self) -> &'static str;
}

/// Pending in-memory OAuth start session (keyed by `state`).
#[derive(Clone, Debug)]
pub struct PendingOAuth {
    pub provider: String,
    pub pkce: Pkce,
    pub created_at: DateTime<Utc>,
}

impl TokenSet {
    /// Build from a typical OAuth token JSON body (`access_token`, `refresh_token`, `expires_in`).
    pub fn from_oauth_json(v: &serde_json::Value) -> anyhow::Result<Self> {
        let access_token = v
            .get("access_token")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow::anyhow!("token response missing access_token"))?
            .to_string();
        let refresh_token = v
            .get("refresh_token")
            .and_then(|x| x.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let expires_at = v
            .get("expires_in")
            .and_then(|x| x.as_i64().or_else(|| x.as_u64().map(|u| u as i64)))
            .filter(|&s| s > 0)
            .map(|secs| Utc::now() + chrono::Duration::seconds(secs));
        Ok(Self {
            access_token,
            refresh_token,
            expires_at,
        })
    }
}
