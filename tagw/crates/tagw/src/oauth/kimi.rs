//! Kimi Code OAuth — primarily device-code; refresh via token URL.
//!
//! Constants from `open-sse/providers/registry/kimi.js`.
//! Start URL points at the device authorize page (no classic auth-code start).

use async_trait::async_trait;

use super::types::{OAuthProvider, Pkce, TokenSet};

pub const KIMI_CLIENT_ID: &str = "17e5f671-d194-4dfb-9706-5516cb48c098";
pub const KIMI_TOKEN_URL: &str = "https://auth.kimi.com/api/oauth/token";
pub const KIMI_DEVICE_CODE_URL: &str = "https://auth.kimi.com/api/oauth/device_authorization";
pub const KIMI_AUTHORIZE_DEVICE_URL: &str = "https://www.kimi.com/code/authorize_device";
pub const KIMI_DEFAULT_BASE_URL: &str = "https://api.kimi.com/coding";

#[derive(Clone, Debug)]
pub struct KimiProvider {
    pub http: reqwest::Client,
    pub client_id: String,
    pub authorize_url: String,
    pub token_url: String,
    pub device_code_url: String,
}

impl Default for KimiProvider {
    fn default() -> Self {
        Self::new(reqwest::Client::new())
    }
}

impl KimiProvider {
    pub fn new(http: reqwest::Client) -> Self {
        Self {
            http,
            client_id: KIMI_CLIENT_ID.into(),
            authorize_url: KIMI_AUTHORIZE_DEVICE_URL.into(),
            token_url: KIMI_TOKEN_URL.into(),
            device_code_url: KIMI_DEVICE_CODE_URL.into(),
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
impl OAuthProvider for KimiProvider {
    fn id(&self) -> &'static str {
        "kimi"
    }

    fn authorize_url(&self, pkce: &Pkce) -> String {
        // Device-code flow: surface the authorize_device page with state for correlation.
        // Full device-code polling is out of scope for start/callback routes; this URL
        // is enough for admins to begin login. Callback can still accept a pasted code
        let mut url = self.authorize_url.clone();
        let sep = if url.contains('?') { '&' } else { '?' };
        url.push(sep);
        url.push_str(&format!(
            "state={}&redirect_uri={}",
            urlencoding::encode(&pkce.state),
            urlencoding::encode(&pkce.redirect_uri)
        ));
        url
    }

    async fn exchange_code(&self, code: &str, _pkce: &Pkce) -> anyhow::Result<TokenSet> {
        // Treat `code` as a device_code / authorization_code depending on grant.
        // Prefer authorization_code shape so mock tests and future browser flow work.
        let body = [
            ("grant_type", "authorization_code"),
            ("client_id", self.client_id.as_str()),
            ("code", code),
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
            anyhow::bail!("kimi token exchange failed ({status}): {text}");
        }
        let v: serde_json::Value = res.json().await?;
        TokenSet::from_oauth_json(&v)
    }

    async fn refresh(&self, refresh_token: &str) -> anyhow::Result<TokenSet> {
        let body = [
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token),
            ("client_id", self.client_id.as_str()),
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
            anyhow::bail!("kimi token refresh failed ({status}): {text}");
        }
        let v: serde_json::Value = res.json().await?;
        let mut tokens = TokenSet::from_oauth_json(&v)?;
        if tokens.refresh_token.is_none() {
            tokens.refresh_token = Some(refresh_token.to_string());
        }
        Ok(tokens)
    }

    fn default_base_url(&self) -> &'static str {
        KIMI_DEFAULT_BASE_URL
    }
}
