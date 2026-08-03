//! API-key providers: CRUD + convert enabled accounts into routing pools.
//!
//! Secrets live in `accounts.credentials_json` (no encrypt-at-rest in v1).
//! On mutate, callers must reload [`crate::cache::ConfigCache`] so the proxy
//! hot path sees the new pool.

use std::collections::HashMap;

use anyhow::{anyhow, Context, Result};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::cache::CachedAccount;
use crate::db::Db;
use crate::router::AccountRef;
use crate::state::DEFAULT_POOL_KEY;

/// Supported API-key provider type strings (schema + admin API).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyProviderType {
    Glm,
    OpenModel,
    Alibaba,
    Anthropic,
    Minimax,
    Kimi,
    Deepseek,
    /// Generic OpenAI-compatible upstream; `base_url` required on accounts.
    OpenaiCompat,
}

impl ApiKeyProviderType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Glm => "glm",
            Self::OpenModel => "open_model",
            Self::Alibaba => "alibaba",
            Self::Anthropic => "anthropic",
            Self::Minimax => "minimax",
            Self::Kimi => "kimi",
            Self::Deepseek => "deepseek",
            Self::OpenaiCompat => "openai_compat",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s.trim() {
            "glm" => Ok(Self::Glm),
            "open_model" => Ok(Self::OpenModel),
            "alibaba" => Ok(Self::Alibaba),
            "anthropic" => Ok(Self::Anthropic),
            "minimax" => Ok(Self::Minimax),
            "kimi" => Ok(Self::Kimi),
            "deepseek" => Ok(Self::Deepseek),
            "openai_compat" => Ok(Self::OpenaiCompat),
            other => Err(anyhow!(
                "unsupported provider_type '{other}'; expected one of: \
                 glm, open_model, alibaba, anthropic, minimax, kimi, deepseek, openai_compat"
            )),
        }
    }

    /// Default OpenAI-compatible origin (path `/v1/*` appended by proxy).
    /// `openai_compat` has no default — admin must supply `base_url`.
    pub fn default_base_url(self) -> Option<&'static str> {
        match self {
            // Coding / OpenAI-compat style origins (best-effort; override via account base_url).
            Self::Glm => Some("https://api.z.ai/api/coding/paas/v4"),
            Self::OpenModel => Some("https://api.openmodel.ai"),
            Self::Alibaba => Some("https://coding-intl.dashscope.aliyuncs.com"),
            Self::Anthropic => Some("https://api.anthropic.com"),
            Self::Minimax => Some("https://api.minimax.io"),
            Self::Kimi => Some("https://api.kimi.com/coding"),
            Self::Deepseek => Some("https://api.deepseek.com"),
            Self::OpenaiCompat => None,
        }
    }
}

/// Stored in `accounts.credentials_json`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiKeyCredentials {
    pub api_key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
}

/// Redacted credentials for admin list/detail responses.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiKeyCredentialsPublic {
    /// Prefix-only view of the secret (never full key).
    pub api_key_prefix: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<String>>,
}

impl ApiKeyCredentials {
    pub fn to_public(&self) -> ApiKeyCredentialsPublic {
        let prefix: String = self.api_key.chars().take(8).collect();
        ApiKeyCredentialsPublic {
            api_key_prefix: if prefix.is_empty() {
                "***".into()
            } else {
                format!("{prefix}…")
            },
            base_url: self.base_url.clone(),
            models: self.models.clone(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderPublic {
    pub id: String,
    pub kind: String,
    pub provider_type: String,
    pub name: String,
    pub enabled: bool,
    pub config_json: Value,
    pub created_at: String,
    pub accounts: Vec<AccountPublic>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountPublic {
    pub id: String,
    pub provider_id: String,
    pub label: String,
    pub enabled: bool,
    pub credentials: ApiKeyCredentialsPublic,
    pub quota_json: Value,
    pub created_at: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreateProviderRequest {
    pub provider_type: String,
    pub name: String,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub config_json: Option<Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct CreateAccountRequest {
    pub label: String,
    pub api_key: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub models: Option<Vec<String>>,
    #[serde(default)]
    pub enabled: Option<bool>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PatchEnabledRequest {
    pub enabled: bool,
}

// ── DB helpers ──────────────────────────────────────────────────────────────

fn parse_config_json(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| Value::Object(Default::default()))
}

fn parse_credentials(raw: &str) -> Result<ApiKeyCredentials> {
    serde_json::from_str(raw).context("parse credentials_json")
}

/// Create an API-key provider row. Always `kind = api_key`.
pub fn create_provider(db: &Db, req: &CreateProviderRequest) -> Result<ProviderPublic> {
    let provider_type = ApiKeyProviderType::parse(&req.provider_type)?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err(anyhow!("name must not be empty"));
    }
    let enabled = req.enabled.unwrap_or(true);
    let config = req
        .config_json
        .clone()
        .unwrap_or_else(|| Value::Object(Default::default()));
    let config_str =
        serde_json::to_string(&config).context("serialize provider config_json")?;
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO providers (id, kind, provider_type, name, enabled, config_json, created_at)
             VALUES (?1, 'api_key', ?2, ?3, ?4, ?5, ?6)",
            params![
                id,
                provider_type.as_str(),
                name,
                if enabled { 1 } else { 0 },
                config_str,
                created_at
            ],
        )?;
        Ok(())
    })
    .context("insert providers")?;

    Ok(ProviderPublic {
        id,
        kind: "api_key".into(),
        provider_type: provider_type.as_str().into(),
        name: name.to_string(),
        enabled,
        config_json: config,
        created_at,
        accounts: vec![],
    })
}

/// List all providers (api_key + oauth) with nested accounts (redacted secrets).
pub fn list_providers(db: &Db) -> Result<Vec<ProviderPublic>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, kind, provider_type, name, enabled, config_json, created_at
             FROM providers
             ORDER BY created_at ASC",
        )?;
        let providers: Vec<(String, String, String, String, i64, String, String)> = stmt
            .query_map([], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let mut out = Vec::with_capacity(providers.len());
        for (id, kind, provider_type, name, enabled, config_json, created_at) in providers {
            let accounts = load_accounts_for_provider(conn, &id)?;
            out.push(ProviderPublic {
                id,
                kind,
                provider_type,
                name,
                enabled: enabled != 0,
                config_json: parse_config_json(&config_json),
                created_at,
                accounts,
            });
        }
        Ok(out)
    })
    .context("list providers")
}

fn load_accounts_for_provider(
    conn: &rusqlite::Connection,
    provider_id: &str,
) -> rusqlite::Result<Vec<AccountPublic>> {
    let mut stmt = conn.prepare(
        "SELECT id, provider_id, label, enabled, credentials_json, quota_json, created_at
         FROM accounts
         WHERE provider_id = ?1
         ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![provider_id], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
            r.get::<_, String>(6)?,
        ))
    })?;
    let mut accounts = Vec::new();
    for row in rows {
        let (id, provider_id, label, enabled, creds_raw, quota_raw, created_at) = row?;
        let credentials = match parse_credentials(&creds_raw) {
            Ok(c) => c.to_public(),
            Err(_) => ApiKeyCredentialsPublic {
                api_key_prefix: "***".into(),
                base_url: None,
                models: None,
            },
        };
        accounts.push(AccountPublic {
            id,
            provider_id,
            label,
            enabled: enabled != 0,
            credentials,
            quota_json: parse_config_json(&quota_raw),
            created_at,
        });
    }
    Ok(accounts)
}

/// Create an account under an existing API-key provider.
pub fn create_account(
    db: &Db,
    provider_id: &str,
    req: &CreateAccountRequest,
) -> Result<AccountPublic> {
    let label = req.label.trim();
    if label.is_empty() {
        return Err(anyhow!("label must not be empty"));
    }
    let api_key = req.api_key.trim();
    if api_key.is_empty() {
        return Err(anyhow!("api_key must not be empty"));
    }

    let (provider_type_str, _provider_enabled) = db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT provider_type, enabled FROM providers WHERE id = ?1 AND kind = 'api_key'",
                params![provider_id],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
        })
        .map_err(|e| {
            if matches!(
                e.downcast_ref::<rusqlite::Error>(),
                Some(rusqlite::Error::QueryReturnedNoRows)
            ) {
                anyhow!("provider {provider_id} not found or not api_key kind")
            } else {
                e
            }
        })
        .context("lookup provider for account create")?;

    let provider_type = ApiKeyProviderType::parse(&provider_type_str)?;

    let base_url = req
        .base_url
        .as_ref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            provider_type
                .default_base_url()
                .map(|s| s.trim_end_matches('/').to_string())
        });

    if base_url.is_none() {
        return Err(anyhow!(
            "base_url is required for provider_type '{}'",
            provider_type.as_str()
        ));
    }

    let credentials = ApiKeyCredentials {
        api_key: api_key.to_string(),
        base_url: base_url.clone(),
        models: req.models.clone(),
    };
    let credentials_json =
        serde_json::to_string(&credentials).context("serialize credentials_json")?;
    let enabled = req.enabled.unwrap_or(true);
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();
    let quota_json = "{}";

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO accounts (id, provider_id, label, enabled, credentials_json, quota_json, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                provider_id,
                label,
                if enabled { 1 } else { 0 },
                credentials_json,
                quota_json,
                created_at
            ],
        )?;
        Ok(())
    })
    .context("insert accounts")?;

    Ok(AccountPublic {
        id,
        provider_id: provider_id.to_string(),
        label: label.to_string(),
        enabled,
        credentials: credentials.to_public(),
        quota_json: Value::Object(Default::default()),
        created_at,
    })
}

/// Enable/disable a provider. Returns false if not found.
pub fn set_provider_enabled(db: &Db, provider_id: &str, enabled: bool) -> Result<bool> {
    let n = db
        .with_conn(|conn| {
            conn.execute(
                "UPDATE providers SET enabled = ?1 WHERE id = ?2",
                params![if enabled { 1 } else { 0 }, provider_id],
            )
        })
        .context("update providers.enabled")?;
    Ok(n > 0)
}

/// Enable/disable an account. Returns false if not found.
pub fn set_account_enabled(
    db: &Db,
    provider_id: &str,
    account_id: &str,
    enabled: bool,
) -> Result<bool> {
    let n = db
        .with_conn(|conn| {
            conn.execute(
                "UPDATE accounts SET enabled = ?1 WHERE id = ?2 AND provider_id = ?3",
                params![if enabled { 1 } else { 0 }, account_id, provider_id],
            )
        })
        .context("update accounts.enabled")?;
    Ok(n > 0)
}

/// Load API-key accounts into routing pools.
///
/// - One pool per `provider_id` (enabled accounts only when both provider+account enabled)
/// - Plus [`DEFAULT_POOL_KEY`] containing all enabled accounts (proxy uses this until model→pool mapping)
///
/// Disabled providers or accounts appear with `enabled: false` only in provider-id pools so
/// diagnostics can still see them; they are **not** added to the default pool as enabled.
pub fn load_account_pools(db: &Db) -> Result<HashMap<String, Vec<CachedAccount>>> {
    let rows = db
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT a.id, a.provider_id, a.enabled, a.credentials_json,
                        p.enabled, p.provider_type
                 FROM accounts a
                 INNER JOIN providers p ON p.id = a.provider_id
                 WHERE p.kind = 'api_key'
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
        })
        .context("load api_key accounts for pools")?;

    let mut by_provider: HashMap<String, Vec<CachedAccount>> = HashMap::new();
    let mut default_pool: Vec<CachedAccount> = Vec::new();

    for (account_id, provider_id, acct_enabled, creds_raw, prov_enabled, provider_type_str) in rows
    {
        let provider_type = match ApiKeyProviderType::parse(&provider_type_str) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(
                    account_id = %account_id,
                    provider_type = %provider_type_str,
                    error = %e,
                    "skip account: unknown provider_type"
                );
                continue;
            }
        };
        let creds = match parse_credentials(&creds_raw) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    account_id = %account_id,
                    error = %e,
                    "skip account: bad credentials_json"
                );
                continue;
            }
        };
        let Some(base) = resolve_base_url(&creds, provider_type) else {
            tracing::warn!(
                account_id = %account_id,
                provider_type = %provider_type_str,
                "skip account: missing base_url"
            );
            continue;
        };

        let enabled = prov_enabled != 0 && acct_enabled != 0;
        let account = AccountRef {
            account_id: account_id.clone(),
            provider_id: provider_id.clone(),
            upstream_base: base.trim_end_matches('/').to_string(),
            auth_header: format!("Bearer {}", creds.api_key),
            is_oauth: false,
        };
        let cached = CachedAccount {
            account: account.clone(),
            enabled,
        };

        by_provider
            .entry(provider_id)
            .or_default()
            .push(cached.clone());

        // Default pool: only effectively enabled accounts (proxy RR).
        if enabled {
            default_pool.push(cached);
        }
    }

    by_provider.insert(DEFAULT_POOL_KEY.to_string(), default_pool);
    Ok(by_provider)
}

fn resolve_base_url(
    creds: &ApiKeyCredentials,
    provider_type: ApiKeyProviderType,
) -> Option<String> {
    if let Some(ref b) = creds.base_url {
        let t = b.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    provider_type
        .default_base_url()
        .map(|s| s.trim_end_matches('/').to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_type_roundtrip() {
        for s in [
            "glm",
            "open_model",
            "alibaba",
            "anthropic",
            "minimax",
            "kimi",
            "deepseek",
            "openai_compat",
        ] {
            let t = ApiKeyProviderType::parse(s).unwrap();
            assert_eq!(t.as_str(), s);
        }
        assert!(ApiKeyProviderType::parse("nope").is_err());
    }

    #[test]
    fn openai_compat_requires_base_url_default_none() {
        assert!(ApiKeyProviderType::OpenaiCompat.default_base_url().is_none());
        assert!(ApiKeyProviderType::Deepseek.default_base_url().is_some());
    }

    #[test]
    fn credentials_redact_prefix() {
        let c = ApiKeyCredentials {
            api_key: "sk-abcdef012345".into(),
            base_url: Some("http://x".into()),
            models: None,
        };
        let p = c.to_public();
        assert!(p.api_key_prefix.starts_with("sk-abcde"));
        assert!(!p.api_key_prefix.contains("012345"));
    }
}
