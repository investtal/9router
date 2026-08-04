//! Available models catalog — 9router-style ids: `provider/model`.

use serde::Serialize;

use crate::cache::ConfigCache;
use crate::db::Db;
use crate::error::AppError;
use crate::providers::api_key::ApiKeyCredentials;
use crate::router::type_pool_key;

/// One model entry for clients (`id` is what you put in `model` field).
#[derive(Clone, Debug, Serialize)]
pub struct ModelEntry {
    /// Client model id, e.g. `glm/glm-5.2`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Canonical provider type (`glm`, `xai`, …).
    pub provider: String,
    /// Upstream model id without prefix.
    pub upstream_model: String,
    pub owned_by: String,
    /// OpenAI `/v1/models` object type.
    pub object: &'static str,
}

/// Default bare model ids per provider type (used when account has no custom list).
pub fn default_models_for_provider(provider_type: &str) -> &'static [&'static str] {
    match provider_type {
        "glm" => &["glm-5.2", "glm-5", "glm-5-turbo", "glm-4.7", "glm-4.6", "glm-4.5"],
        "xai" => &[
            "grok-4.5",
            "grok-4",
            "grok-4-fast-reasoning",
            "grok-code-fast-1",
            "grok-3",
        ],
        "codex" => &[
            "gpt-5.4",
            "gpt-5.3-codex",
            "gpt-5.2",
            "gpt-4.1",
            "gpt-4o",
            "o3",
            "o4-mini",
        ],
        "deepseek" => &["deepseek-chat", "deepseek-reasoner"],
        "minimax" => &["MiniMax-M2.5", "MiniMax-Text-01"],
        "alibaba" => &["qwen-max", "qwen-plus", "qwen-turbo"],
        "kimi" => &["kimi-k2", "moonshot-v1-128k"],
        "anthropic" => &[
            "claude-sonnet-4-20250514",
            "claude-opus-4-20250514",
            "claude-haiku-4-5-20251001",
        ],
        "antigravity" => &[
            "claude-sonnet-4-6",
            "claude-opus-4-6-thinking",
            "gemini-3-flash",
            "gemini-3.1-pro-low",
        ],
        "open_model" => &["default"],
        _ => &[],
    }
}

/// Client-facing prefix for a provider type (what goes before `/` in model id).
pub fn client_prefix_for_provider(provider_type: &str) -> &str {
    match provider_type {
        "codex" => "codex",
        "openai" | "openai_compat" => "openai",
        other => other,
    }
}

/// Build catalog from enabled accounts in cache + DB credentials models lists.
pub fn list_available_models(db: &Db, cache: &ConfigCache) -> Result<Vec<ModelEntry>, AppError> {
    use std::collections::{BTreeMap, BTreeSet};

    // provider_type → set of bare model ids
    let mut by_type: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();

    // Which types have at least one enabled account in cache pools?
    let types_with_accounts: Vec<String> = {
        let mut types = BTreeSet::new();
        // Scan known types from enum + oauth types.
        let candidates = [
            "glm",
            "xai",
            "codex",
            "deepseek",
            "minimax",
            "alibaba",
            "kimi",
            "anthropic",
            "antigravity",
            "open_model",
            "openai_compat",
        ];
        for t in candidates {
            if !cache.enabled_accounts(&type_pool_key(t)).is_empty() {
                types.insert(t.to_string());
            }
            // Also check anthropic pool for claude/anthropic/antigravity
            if matches!(t, "anthropic" | "antigravity")
                && !cache
                    .enabled_accounts(crate::state::ANTHROPIC_POOL_KEY)
                    .is_empty()
            {
                types.insert(t.to_string());
            }
        }
        // OAuth antigravity / claude may only appear as type:antigravity or type:claude
        if !cache.enabled_accounts(&type_pool_key("claude")).is_empty() {
            types.insert("antigravity".into()); // map claude oauth under antigravity-style list too
            types.insert("anthropic".into());
        }
        types.into_iter().collect()
    };

    for ptype in &types_with_accounts {
        let set = by_type.entry(ptype.clone()).or_default();
        for m in default_models_for_provider(ptype) {
            set.insert((*m).to_string());
        }
    }

    // Merge custom models from account credentials_json
    let custom: Vec<(String, Option<String>)> = db
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT p.provider_type, a.credentials_json
                 FROM accounts a
                 JOIN providers p ON p.id = a.provider_id
                 WHERE a.enabled = 1 AND p.enabled = 1",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (ptype, creds) = row?;
                out.push((ptype, Some(creds)));
            }
            Ok(out)
        })
        .map_err(AppError::Internal)?;

    for (ptype, creds_json) in custom {
        // Every enabled DB account contributes its type + defaults (OAuth antigravity/claude
        // often have no models[] in credentials, so catalog must seed defaults here).
        let catalog_type = match ptype.as_str() {
            "claude" => "anthropic".to_string(),
            other => other.to_string(),
        };
        {
            let set = by_type.entry(catalog_type.clone()).or_default();
            for m in default_models_for_provider(&catalog_type) {
                set.insert((*m).to_string());
            }
            // Antigravity OAuth also exposes Claude-family + Gemini catalog entries.
            if ptype == "antigravity" {
                for m in default_models_for_provider("antigravity") {
                    set.insert((*m).to_string());
                }
            }
        }
        let Some(json) = creds_json else { continue };
        // Try api_key credentials shape first (optional models override/extend).
        if let Ok(creds) = serde_json::from_str::<ApiKeyCredentials>(&json) {
            if let Some(models) = creds.models {
                let set = by_type.entry(catalog_type).or_default();
                for m in models {
                    if !m.trim().is_empty() {
                        set.insert(m.trim().to_string());
                    }
                }
            }
        }
    }

    // If anthropic pool has accounts but type map empty, add defaults
    if !cache
        .enabled_accounts(crate::state::ANTHROPIC_POOL_KEY)
        .is_empty()
    {
        for t in ["anthropic", "antigravity"] {
            let set = by_type.entry(t.into()).or_default();
            for m in default_models_for_provider(t) {
                set.insert((*m).to_string());
            }
        }
    }

    let mut entries = Vec::new();
    for (ptype, models) in by_type {
        // Skip empty model sets with no defaults
        if models.is_empty() {
            continue;
        }
        // Skip pure openai_compat without explicit models (no useful catalog)
        if ptype == "openai_compat" && models.is_empty() {
            continue;
        }
        let prefix = client_prefix_for_provider(&ptype);
        for bare in models {
            // Don't double-prefix if admin stored already-prefixed ids
            let (id, upstream) = if bare.contains('/') {
                (bare.clone(), bare.split_once('/').map(|(_, r)| r.to_string()).unwrap_or(bare.clone()))
            } else {
                (format!("{prefix}/{bare}"), bare.clone())
            };
            entries.push(ModelEntry {
                id: id.clone(),
                name: bare.clone(),
                provider: ptype.clone(),
                upstream_model: upstream,
                owned_by: ptype.clone(),
                object: "model",
            });
        }
    }

    entries.sort_by(|a, b| a.id.cmp(&b.id));
    entries.dedup_by(|a, b| a.id == b.id);
    Ok(entries)
}

/// OpenAI-compatible list response.
#[derive(Clone, Debug, Serialize)]
pub struct OpenAiModelsList {
    pub object: &'static str,
    pub data: Vec<OpenAiModelObject>,
}

#[derive(Clone, Debug, Serialize)]
pub struct OpenAiModelObject {
    pub id: String,
    pub object: &'static str,
    pub created: i64,
    pub owned_by: String,
}

pub fn to_openai_models_list(entries: &[ModelEntry]) -> OpenAiModelsList {
    let created = chrono::Utc::now().timestamp();
    OpenAiModelsList {
        object: "list",
        data: entries
            .iter()
            .map(|e| OpenAiModelObject {
                id: e.id.clone(),
                object: "model",
                created,
                owned_by: e.owned_by.clone(),
            })
            .collect(),
    }
}

