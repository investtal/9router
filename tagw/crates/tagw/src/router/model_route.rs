//! Model-name → routing pool key resolution for OpenAI-compat proxy.

use crate::cache::ConfigCache;
use crate::state::OPENAI_COMPAT_POOL_KEY;

/// Pool key for a provider type synthetic RR set, e.g. `"type:glm"`.
pub fn type_pool_key(provider_type: &str) -> String {
    format!("type:{provider_type}")
}

/// Resolve which OpenAI-compat account pool to use for a request model.
///
/// Prefer a type-specific pool when the model name matches a known family **and**
/// that pool has at least one enabled account; otherwise fall back to
/// [`OPENAI_COMPAT_POOL_KEY`].
///
/// Matching is case-insensitive substring (contains). Fail-over stays within the
/// chosen pool_key (caller uses the same key for RR + retries).
pub fn resolve_openai_pool_key(model: Option<&str>, cache: &ConfigCache) -> String {
    let Some(model) = model.map(str::trim).filter(|s| !s.is_empty()) else {
        return OPENAI_COMPAT_POOL_KEY.to_string();
    };
    let m = model.to_ascii_lowercase();

    // (needles, preferred type pool key). First matching family with a non-empty
    let type_routes: &[(&[&str], &str)] = &[
        (&["glm", "zai"], "glm"),
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

    // OpenAI-family / codex OAuth live on the aggregate openai_compat pool.
    if ["gpt", "o1", "o3", "o4", "codex"]
        .iter()
        .any(|n| m.contains(n))
    {
        return OPENAI_COMPAT_POOL_KEY.to_string();
    }

    OPENAI_COMPAT_POOL_KEY.to_string()
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
    fn no_model_uses_openai_compat() {
        let cache = ConfigCache::new();
        assert_eq!(
            resolve_openai_pool_key(None, &cache),
            OPENAI_COMPAT_POOL_KEY
        );
        assert_eq!(
            resolve_openai_pool_key(Some(""), &cache),
            OPENAI_COMPAT_POOL_KEY
        );
    }

    #[test]
    fn glm_model_routes_to_type_glm_when_populated() {
        let cache = ConfigCache::new();
        cache.set_account_pool(type_pool_key("glm"), vec![acct("g1", "prov-glm")]);
        // Aggregate empty on purpose — type pool alone must be selected.
        assert!(cache.enabled_accounts(OPENAI_COMPAT_POOL_KEY).is_empty());
        assert_eq!(
            resolve_openai_pool_key(Some("glm-4"), &cache),
            type_pool_key("glm")
        );
        assert_eq!(
            resolve_openai_pool_key(Some("GLM-4-Flash"), &cache),
            type_pool_key("glm")
        );
        assert_eq!(
            resolve_openai_pool_key(Some("zai-coding-v1"), &cache),
            type_pool_key("glm")
        );
    }

    #[test]
    fn glm_model_falls_back_when_type_pool_empty() {
        let cache = ConfigCache::new();
        cache.set_account_pool(OPENAI_COMPAT_POOL_KEY, vec![acct("o1", "prov-oai")]);
        assert_eq!(
            resolve_openai_pool_key(Some("glm-4"), &cache),
            OPENAI_COMPAT_POOL_KEY
        );
    }

    #[test]
    fn deepseek_minimax_alibaba_kimi_routes() {
        let cache = ConfigCache::new();
        cache.set_account_pool(type_pool_key("deepseek"), vec![acct("d1", "p-ds")]);
        cache.set_account_pool(type_pool_key("minimax"), vec![acct("m1", "p-mm")]);
        cache.set_account_pool(type_pool_key("alibaba"), vec![acct("a1", "p-ali")]);
        cache.set_account_pool(type_pool_key("kimi"), vec![acct("k1", "p-kimi")]);

        assert_eq!(
            resolve_openai_pool_key(Some("deepseek-chat"), &cache),
            type_pool_key("deepseek")
        );
        assert_eq!(
            resolve_openai_pool_key(Some("minimax-text-01"), &cache),
            type_pool_key("minimax")
        );
        assert_eq!(
            resolve_openai_pool_key(Some("qwen-max"), &cache),
            type_pool_key("alibaba")
        );
        assert_eq!(
            resolve_openai_pool_key(Some("moonshot-v1"), &cache),
            type_pool_key("kimi")
        );
        assert_eq!(
            resolve_openai_pool_key(Some("kimi-k2"), &cache),
            type_pool_key("kimi")
        );
    }

    #[test]
    fn openai_family_uses_aggregate() {
        let cache = ConfigCache::new();
        cache.set_account_pool(type_pool_key("glm"), vec![acct("g1", "p-glm")]);
        cache.set_account_pool(OPENAI_COMPAT_POOL_KEY, vec![acct("o1", "p-oai")]);
        assert_eq!(
            resolve_openai_pool_key(Some("gpt-4o"), &cache),
            OPENAI_COMPAT_POOL_KEY
        );
        assert_eq!(
            resolve_openai_pool_key(Some("o3-mini"), &cache),
            OPENAI_COMPAT_POOL_KEY
        );
        assert_eq!(
            resolve_openai_pool_key(Some("codex-mini"), &cache),
            OPENAI_COMPAT_POOL_KEY
        );
    }

    #[test]
    fn unknown_model_uses_openai_compat() {
        let cache = ConfigCache::new();
        assert_eq!(
            resolve_openai_pool_key(Some("custom-local-7b"), &cache),
            OPENAI_COMPAT_POOL_KEY
        );
    }

    #[test]
    fn type_pool_key_format() {
        assert_eq!(type_pool_key("glm"), "type:glm");
        assert_eq!(type_pool_key("deepseek"), "type:deepseek");
    }
}
