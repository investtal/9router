//! Claude (Anthropic) OAuth — PKCE with JSON token endpoint.
//!
//! Constants from `open-sse/providers/registry/claude.js` and refresh from
//! `tokenRefresh/providers.js` (`refreshClaudeOAuthToken`).

use async_trait::async_trait;
use serde_json::json;

use super::types::{OAuthProvider, Pkce, TokenSet};

pub const CLAUDE_CLIENT_ID: &str = "9d1c250a-e61b-44d9-88ed-5944d1962f5e";
pub const CLAUDE_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
pub const CLAUDE_TOKEN_URL: &str = "https://api.anthropic.com/v1/oauth/token";
pub const CLAUDE_SCOPES: &str = "org:create_api_key user:profile user:inference";
pub const CLAUDE_DEFAULT_BASE_URL: &str = "https://api.anthropic.com";

#[derive(Clone, Debug)]
pub struct ClaudeProvider {
    pub http: reqwest::Client,
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
}

impl Default for ClaudeProvider {
    fn default() -> Self {
        Self::new(reqwest::Client::new())
    }
}

impl ClaudeProvider {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            client_id: CLAUDE_CLIENT_ID.into(),
            authorize_url: CLAUDE_AUTHORIZE_URL.into(),
            token_url: CLAUDE_TOKEN_URL.into(),
        }
    }

    pub fn with_endpoints(mut self, token_url: impl Into<String>, authorize_url: Option<String>) -> Self {
        self.token_url = token_url.into();
        if let Some(a) = authorize_url {
            self.authorize_url = a;
        }
        self
    }
}

#[async_trait]
impl OAuthProvider for ClaudeProvider {
    fn id(&self) -> &'static str {
        "claude"
    }

    fn authorize_url(&self, pkce: &Pkce) -> String {
        // Mirrors ClaudeService.buildClaudeAuthUrl (code=true + PKCE).
        let params = [
            ("code", "true"),
            ("client_id", self.client_id.as_str()),
            ("response_type", "code"),
            ("redirect_uri", pkce.redirect_uri.as_str()),
            ("scope", CLAUDE_SCOPES),
            ("code_challenge", pkce.code_challenge.as_str()),
            ("code_challenge_method", "S256"),
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
        // Claude may return `code#state` in the callback fragment path; strip.
        let (auth_code, code_state) = if let Some((c, s)) = code.split_once('#') {
            (c, s)
        } else {
            (code, pkce.state.as_str())
        };
        let res = self
            .http
            .post(&self.token_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&json!({
                "code": auth_code,
                "state": code_state,
                "grant_type": "authorization_code",
                "client_id": self.client_id,
                "redirect_uri": pkce.redirect_uri,
                "code_verifier": pkce.code_verifier,
            }))
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            anyhow::bail!("claude token exchange failed ({status}): {text}");
        }
        let v: serde_json::Value = res.json().await?;
        TokenSet::from_oauth_json(&v)
    }

    async fn refresh(&self, refresh_token: &str) -> anyhow::Result<TokenSet> {
        let res = self
            .http
            .post(&self.token_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&json!({
                "grant_type": "refresh_token",
                "refresh_token": refresh_token,
                "client_id": self.client_id,
            }))
            .send()
            .await?;
        if !res.status().is_success() {
            let status = res.status();
            let text = res.text().await.unwrap_or_default();
            anyhow::bail!("claude token refresh failed ({status}): {text}");
        }
        let v: serde_json::Value = res.json().await?;
        let mut tokens = TokenSet::from_oauth_json(&v)?;
        if tokens.refresh_token.is_none() {
            tokens.refresh_token = Some(refresh_token.to_string());
        }
        Ok(tokens)
    }

    fn default_base_url(&self) -> &'static str {
        CLAUDE_DEFAULT_BASE_URL
    }
}
