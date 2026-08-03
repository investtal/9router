//! Antigravity (Google) OAuth — standard auth code + client secret (no PKCE required).
//!
//! Constants from `open-sse/providers/registry/antigravity.js` + `ANTIGRAVITY_OAUTH_CLIENT`.

use async_trait::async_trait;

use super::types::{OAuthProvider, Pkce, TokenSet};

pub const ANTIGRAVITY_CLIENT_ID: &str =
    "1071006060591-tmhssin2h21lcre235vtolojh4g403ep.apps.googleusercontent.com";
pub const ANTIGRAVITY_CLIENT_SECRET: &str = "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";
pub const ANTIGRAVITY_AUTHORIZE_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const ANTIGRAVITY_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
pub const ANTIGRAVITY_SCOPES: &str = "https://www.googleapis.com/auth/cloud-platform \
https://www.googleapis.com/auth/userinfo.email \
https://www.googleapis.com/auth/userinfo.profile \
https://www.googleapis.com/auth/cclog \
https://www.googleapis.com/auth/experimentsandconfigs";
pub const ANTIGRAVITY_DEFAULT_BASE_URL: &str = "https://cloudcode-pa.googleapis.com";

#[derive(Clone, Debug)]
pub struct AntigravityProvider {
    pub http: reqwest::Client,
    pub client_id: String,
    pub client_secret: String,
    pub authorize_url: String,
    pub token_url: String,
}

impl Default for AntigravityProvider {
    fn default() -> Self {
        Self::new(reqwest::Client::new())
    }
}

impl AntigravityProvider {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            client_id: ANTIGRAVITY_CLIENT_ID.into(),
            client_secret: ANTIGRAVITY_CLIENT_SECRET.into(),
            authorize_url: ANTIGRAVITY_AUTHORIZE_URL.into(),
            token_url: ANTIGRAVITY_TOKEN_URL.into(),
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
impl OAuthProvider for AntigravityProvider {
    fn id(&self) -> &'static str {
        "antigravity"
    }

    fn authorize_url(&self, pkce: &Pkce) -> String {
        // Google OAuth; PKCE optional — still pass challenge when present.
        let params = [
            ("client_id", self.client_id.as_str()),
            ("response_type", "code"),
            ("redirect_uri", pkce.redirect_uri.as_str()),
            ("scope", ANTIGRAVITY_SCOPES),
            ("state", pkce.state.as_str()),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("code_challenge", pkce.code_challenge.as_str()),
            ("code_challenge_method", "S256"),
        ];
        let qs = params
            .iter()
            .map(|(k, v)| format!("{}={}", k, urlencoding::encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        format!("{}?{}", self.authorize_url, qs)
    }

    async fn exchange_code(&self, code: &str, pkce: &Pkce) -> anyhow::Result<TokenSet> {
        let body = [
            ("grant_type", "authorization_code"),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", pkce.redirect_uri.as_str()),
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
            anyhow::bail!("antigravity token exchange failed ({status}): {text}");
        }
        let v: serde_json::Value = res.json().await?;
        TokenSet::from_oauth_json(&v)
    }

    async fn refresh(&self, refresh_token: &str) -> anyhow::Result<TokenSet> {
        let body = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.as_str()),
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
            anyhow::bail!("antigravity token refresh failed ({status}): {text}");
        }
        let v: serde_json::Value = res.json().await?;
        let mut tokens = TokenSet::from_oauth_json(&v)?;
        if tokens.refresh_token.is_none() {
            tokens.refresh_token = Some(refresh_token.to_string());
        }
        Ok(tokens)
    }

    fn default_base_url(&self) -> &'static str {
        ANTIGRAVITY_DEFAULT_BASE_URL
    }
}
