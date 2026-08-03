//! Codex (OpenAI ChatGPT) OAuth — authorization code + PKCE.
//!
//! Endpoints ported from 9router `open-sse/providers/registry/codex.js` and
//! token refresh from `open-sse/services/tokenRefresh/providers.js`.

use async_trait::async_trait;
use serde_json::json;

use super::types::{OAuthProvider, Pkce, TokenSet};

/// Default public Codex CLI client id (from registry).
pub const CODEX_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const CODEX_AUTHORIZE_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const CODEX_TOKEN_URL: &str = "https://auth.openai.com/oauth/token";
pub const CODEX_SCOPE: &str = "openid profile email offline_access";
/// Upstream used for OpenAI-compatible proxy hops (override via credentials.base_url).
pub const CODEX_DEFAULT_BASE_URL: &str = "https://api.openai.com";

#[derive(Clone, Debug)]
pub struct CodexProvider {
    pub http: reqwest::Client,
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
}

impl Default for CodexProvider {
    fn default() -> Self {
        Self::new(reqwest::Client::new())
    }
}

impl CodexProvider {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            client_id: CODEX_CLIENT_ID.into(),
            authorize_url: CODEX_AUTHORIZE_URL.into(),
            token_url: CODEX_TOKEN_URL.into(),
        }
    }

    /// Point token (and optionally authorize) endpoints at a mock / override.
    pub fn with_endpoints(mut self, token_url: impl Into<String>, authorize_url: Option<String>) -> Self {
        self.token_url = token_url.into();
        if let Some(a) = authorize_url {
            self.authorize_url = a;
        }
        self
    }

    pub fn with_client_id(mut self, client_id: impl Into<String>) -> Self {
        self.client_id = client_id.into();
        self
    }
}

#[async_trait]
impl OAuthProvider for CodexProvider {
    fn id(&self) -> &'static str {
        "codex"
    }

    fn authorize_url(&self, pkce: &Pkce) -> String {
        // Encode spaces as %20 (not +) — matches 9router CodexService.buildCodexAuthUrl.
        let params = [
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", pkce.redirect_uri.as_str()),
            ("scope", CODEX_SCOPE),
            ("code_challenge", pkce.code_challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", "codex_cli_rs"),
            ("state", pkce.state.as_str()),
        ];
        let qs = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        format!("{}?{}", self.authorize_url, qs)
    }

    async fn exchange_code(&self, code: &str, pkce: &Pkce) -> anyhow::Result<TokenSet> {
        // Codex exchange uses form-urlencoded (9router oauth.js / codex.js).
        let body = [
            ("grant_type", "authorization_code"),
            ("client_id", self.client_id.as_str()),
            ("code", code),
            ("redirect_uri", pkce.redirect_uri.as_str()),
            ("code_verifier", pkce.code_verifier.as_str()),
        ];
        let res = self
            .http
            .post(&self.token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("Accept", "application/json")
            .form(&body)
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            anyhow::bail!("codex token exchange failed ({status}): {text}");
        }
        let v: serde_json::Value = res.json().await?;
        TokenSet::from_oauth_json(&v)
    }

    async fn refresh(&self, refresh_token: &str) -> anyhow::Result<TokenSet> {
        // Production refresh uses JSON body (tokenRefresh/providers.js refreshCodexToken).
        let res = self
            .http
            .post(&self.token_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&json!({
                "client_id": self.client_id,
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
            }))
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            anyhow::bail!("codex token refresh failed ({status}): {text}");
        }
        let v: serde_json::Value = res.json().await?;
        let mut tokens = TokenSet::from_oauth_json(&v)?;
        // Keep prior refresh token if provider did not rotate.
        if tokens.refresh_token.is_none() {
            tokens.refresh_token = Some(refresh_token.to_string());
        }
        Ok(tokens)
    }

    fn default_base_url(&self) -> &'static str {
        CODEX_DEFAULT_BASE_URL
    }
}
