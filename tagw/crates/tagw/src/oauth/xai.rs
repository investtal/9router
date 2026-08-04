//! xAI (Grok) OAuth — PKCE public client.
//!
//! Constants from `src/lib/oauth/constants/xai.js` / registry `xai.js`.

use async_trait::async_trait;

use super::types::{OAuthProvider, Pkce, TokenSet};

pub const XAI_CLIENT_ID: &str = "b1a00492-073a-47ea-816f-4c329264a828";
pub const XAI_AUTHORIZE_URL: &str = "https://auth.x.ai/oauth2/authorize";
pub const XAI_TOKEN_URL: &str = "https://auth.x.ai/oauth2/token";
pub const XAI_SCOPE: &str = "openid profile email offline_access grok-cli:access api:access";
pub const XAI_DEFAULT_BASE_URL: &str = "https://api.x.ai";

#[derive(Clone, Debug)]
pub struct XaiProvider {
    pub http: reqwest::Client,
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
}

impl Default for XaiProvider {
    fn default() -> Self {
        Self::new(reqwest::Client::new())
    }
}

impl XaiProvider {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            client_id: XAI_CLIENT_ID.into(),
            authorize_url: XAI_AUTHORIZE_URL.into(),
            token_url: XAI_TOKEN_URL.into(),
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
impl OAuthProvider for XaiProvider {
    fn id(&self) -> &'static str {
        "xai"
    }

    fn authorize_url(&self, pkce: &Pkce) -> String {
        let nonce = {
            use rand_core::{OsRng, RngCore};
            let mut b = [0u8; 16];
            OsRng.fill_bytes(&mut b);
            hex::encode_simple(&b)
        };
        let params = [
            ("response_type", "code"),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", pkce.redirect_uri.as_str()),
            ("scope", XAI_SCOPE),
            ("code_challenge", pkce.code_challenge.as_str()),
            ("code_challenge_method", "S256"),
            ("state", pkce.state.as_str()),
            ("nonce", nonce.as_str()),
            ("plan", "generic"),
            ("referrer", "cli-proxy-api"),
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
            anyhow::bail!("xai token exchange failed ({status}): {text}");
        }
        let v: serde_json::Value = res.json().await?;
        TokenSet::from_oauth_json(&v)
    }

    async fn refresh(&self, refresh_token: &str) -> anyhow::Result<TokenSet> {
        let body = [
            ("grant_type", "refresh_token"),
            ("client_id", self.client_id.as_str()),
            ("refresh_token", refresh_token),
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
            anyhow::bail!("xai token refresh failed ({status}): {text}");
        }
        let v: serde_json::Value = res.json().await?;
        let mut tokens = TokenSet::from_oauth_json(&v)?;
        if tokens.refresh_token.is_none() {
            tokens.refresh_token = Some(refresh_token.to_string());
        }
        Ok(tokens)
    }

    fn default_base_url(&self) -> &'static str {
        XAI_DEFAULT_BASE_URL
    }
}

/// Tiny hex encoder without pulling in the `hex` crate.
mod hex {
    pub fn encode_simple(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(bytes.len() * 2);
        for &b in bytes {
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0xf) as usize] as char);
        }
        out
    }
}
