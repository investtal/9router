//! Model naming & routing — 9router-style `provider/model` (e.g. `glm/glm-5.2`, `xai/grok-4`).

use crate::cache::ConfigCache;
use crate::state::{ANTHROPIC_POOL_KEY, OPENAI_COMPAT_POOL_KEY};

/// Pool key for a provider type synthetic RR set, e.g. `"type:glm"`.
pub fn type_pool_key(provider_type: &str) -> String {
    format!("type:{provider_type}")
}

/// Result of parsing a client model string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedModel {
    /// Canonical provider type for routing (`glm`, `xai`, `codex`, `anthropic`, …).
    pub provider: Option<String>,
    /// Model id sent **upstream** (prefix stripped).
    pub upstream_model: String,
    /// Original client model string (for logs / usage).
    pub client_model: String,
}

/// Map short aliases → canonical provider type (9router registry style).
pub fn resolve_provider_alias(alias_or_id: &str) -> &str {
    match alias_or_id.to_ascii_lowercase().as_str() {
        "glm" | "zai" | "z.ai" => "glm",
        "xai" | "grok" | "x-ai" => "xai",
        "codex" | "openai" | "oai" => "codex",
        "deepseek" | "ds" => "deepseek",
        "minimax" | "mm" => "minimax",
        "alibaba" | "qwen" | "dashscope" => "alibaba",
        "kimi" | "moonshot" => "kimi",
        "anthropic" | "claude" => "anthropic",
        "antigravity" | "ag" | "google" => "antigravity",
        "open_model" | "openmodel" => "open_model",
        "openai_compat" | "compat" => "openai_compat",
        other => {
            // Leak to static: return via Box for unknown? Use owned elsewhere.
            // For known path we only call with static-ish; return as-is via match default.
            // Can't return &str of local — use identity on input:
            let _ = other;
            alias_or_id
        }
    }
}

/// Parse `provider/model` or bare model name (9router `parseModel`).
pub fn parse_model(model_str: &str) -> ParsedModel {
    let client_model = model_str.trim().to_string();
    if client_model.is_empty() {
        return ParsedModel {
            provider: None,
            upstream_model: String::new(),
            client_model,
        };
    }
    if let Some((prefix, rest)) = client_model.split_once('/') {
        let prefix = prefix.trim();
        let rest = rest.trim();
        if !prefix.is_empty() && !rest.is_empty() {
            let provider = resolve_provider_alias(prefix).to_string();
            // If alias map returned the same non-canonical for unknown, still use lowercased prefix.
            let provider = if provider == prefix {
                prefix.to_ascii_lowercase()
            } else {
                provider
            };
            return ParsedModel {
                provider: Some(provider),
                upstream_model: rest.to_string(),
                client_model,
            };
        }
    }
    ParsedModel {
        provider: None,
        upstream_model: client_model.clone(),
        client_model,
    }
}

/// Resolve which OpenAI-compat account pool to use for a request model.
///
/// Priority:
/// 1. Explicit `provider/model` → `type:{provider}` if that pool has accounts
/// 2. Substring family heuristics (legacy bare names like `glm-5.2`)
/// 3. [`OPENAI_COMPAT_POOL_KEY`]
pub fn resolve_openai_pool_key(model: Option<&str>, cache: &ConfigCache) -> String {
    let Some(model) = model.map(str::trim).filter(|s| !s.is_empty()) else {
        return OPENAI_COMPAT_POOL_KEY.to_string();
    };
    let parsed = parse_model(model);

    if let Some(ref provider) = parsed.provider {
        // Anthropic-family prefixes should not pick OpenAI pools.
        if matches!(provider.as_str(), "anthropic" | "antigravity" | "claude") {
            if !cache.enabled_accounts(ANTHROPIC_POOL_KEY).is_empty() {
                return ANTHROPIC_POOL_KEY.to_string();
            }
        }
        let key = type_pool_key(provider);
        if !cache.enabled_accounts(&key).is_empty() {
            return key;
        }
        // Fallback: codex/openai models share openai_compat aggregate.
        if matches!(provider.as_str(), "codex" | "openai" | "openai_compat") {
            return OPENAI_COMPAT_POOL_KEY.to_string();
        }
        // Prefer type pool even if empty so fail-over errors clearly (no wrong provider).
        if cache.enabled_accounts(OPENAI_COMPAT_POOL_KEY).is_empty() {
            return key;
        }
        return OPENAI_COMPAT_POOL_KEY.to_string();
    }

    // Bare model — family heuristics.
    let m = parsed.upstream_model.to_ascii_lowercase();
    let type_routes: &[(&[&str], &str)] = &[
        (&["glm", "zai"], "glm"),
        (&["grok"], "xai"),
        (&["deepseek"], "deepseek"),
        (&["minimax"], "minimax"),
        (&["qwen", "alibaba"], "alibaba"),
        (&["kimi", "moonshot"], "kimi"),
    ];
    for (needles, ptype) in type_routes {
        if needles.iter().any(|n| m.contains(n)) {
            let key = type_pool_key(ptype);
            if !cache.enabled_accounts(&key).is_empty() {
                return key;
            }
            return OPENAI_COMPAT_POOL_KEY.to_string();
        }
    }
    if ["gpt", "o1", "o3", "o4", "codex"]
        .iter()
        .any(|n| m.contains(n))
    {
        return OPENAI_COMPAT_POOL_KEY.to_string();
    }
    OPENAI_COMPAT_POOL_KEY.to_string()
}

/// Rewrite JSON body `model` field to upstream id (strip `provider/` prefix).
pub fn rewrite_body_model(body: &[u8], upstream_model: &str) -> Option<bytes::Bytes> {
    if body.is_empty() || upstream_model.is_empty() {
        return None;
    }
    let mut v: serde_json::Value = serde_json::from_slice(body).ok()?;
    let obj = v.as_object_mut()?;
    let current = obj.get("model").and_then(|m| m.as_str())?;
    if current == upstream_model {
        return None;
    }
    obj.insert(
        "model".into(),
        serde_json::Value::String(upstream_model.to_string()),
    );
    serde_json::to_vec(&v).ok().map(bytes::Bytes::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::CachedAccount;
    use crate::router::AccountRef;

    fn acct(id: &str, provider_id: &str) -> CachedAccount {
        CachedAccount {
            account: AccountRef {
                account_id: id.into(),
                provider_id: provider_id.into(),
                upstream_base: format!("http://{id}.example"),
                auth_header: format!("Bearer {id}"),
                is_oauth: false,
            },
            enabled: true,
        }
    }

    #[test]
    fn parse_provider_slash_model() {
        let p = parse_model("glm/glm-5.2");
        assert_eq!(p.provider.as_deref(), Some("glm"));
        assert_eq!(p.upstream_model, "glm-5.2");
        assert_eq!(p.client_model, "glm/glm-5.2");

        let p = parse_model("xai/grok-4.5");
        assert_eq!(p.provider.as_deref(), Some("xai"));
        assert_eq!(p.upstream_model, "grok-4.5");

        let p = parse_model("zai/glm-5.2");
        assert_eq!(p.provider.as_deref(), Some("glm"));
    }

    #[test]
    fn parse_bare_model() {
        let p = parse_model("glm-5.2");
        assert_eq!(p.provider, None);
        assert_eq!(p.upstream_model, "glm-5.2");
    }

    #[test]
    fn explicit_prefix_routes_to_type_pool() {
        let cache = ConfigCache::new();
        cache.set_account_pool(type_pool_key("glm"), vec![acct("g1", "prov-glm")]);
        cache.set_account_pool(type_pool_key("xai"), vec![acct("x1", "prov-xai")]);
        assert_eq!(
            resolve_openai_pool_key(Some("glm/glm-5.2"), &cache),
            type_pool_key("glm")
        );
        assert_eq!(
            resolve_openai_pool_key(Some("xai/grok-4"), &cache),
            type_pool_key("xai")
        );
    }

    #[test]
    fn bare_glm_still_routes() {
        let cache = ConfigCache::new();
        cache.set_account_pool(type_pool_key("glm"), vec![acct("g1", "prov-glm")]);
        assert_eq!(
            resolve_openai_pool_key(Some("glm-4"), &cache),
            type_pool_key("glm")
        );
    }

    #[test]
    fn rewrite_strips_prefix() {
        let body = br#"{"model":"glm/glm-5.2","messages":[]}"#;
        let out = rewrite_body_model(body, "glm-5.2").expect("rewrite");
        let v: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(v["model"], "glm-5.2");
    }

    #[test]
    fn no_model_uses_openai_compat() {
        let cache = ConfigCache::new();
        assert_eq!(
            resolve_openai_pool_key(None, &cache),
            OPENAI_COMPAT_POOL_KEY
        );
    }

    #[test]
    fn type_pool_key_format() {
        assert_eq!(type_pool_key("glm"), "type:glm");
    }
}
