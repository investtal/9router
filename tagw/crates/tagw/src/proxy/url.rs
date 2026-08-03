//! Upstream URL joining for versioned OpenAI-compat bases (e.g. z.ai `…/v4`).

/// Join provider `base` with the client `path_and_query`.
///
/// OpenAI SDK clients always send `/v1/chat/completions`. Providers that already
/// bake the version into the base (`…/paas/v4`, `…/v1`) must **not** receive a
/// second `/v1` segment (`…/v4/v1/chat/completions` → 404).
///
/// We only strip when the base ends with `/v{digit}` (e.g. `…/v4`). Bases like
/// `https://api.anthropic.com` or `https://api.z.ai/api/anthropic` keep `/v1/…`.
pub fn join_upstream_url_owned(base: &str, path_and_query: &str) -> String {
    let base = base.trim_end_matches('/');
    if base_ends_with_api_version(base) {
        if let Some(rest) = path_and_query.strip_prefix("/v1/") {
            return format!("{base}/{rest}");
        }
        if let Some(rest) = path_and_query.strip_prefix("/v1?") {
            return format!("{base}?{rest}");
        }
        if path_and_query == "/v1" {
            return base.to_string();
        }
    }
    let path = if path_and_query.starts_with('/') {
        path_and_query
    } else {
        return format!("{base}/{path_and_query}");
    };
    format!("{base}{path}")
}

fn base_ends_with_api_version(base: &str) -> bool {
    let b = base.as_bytes();
    if b.len() < 3 {
        return false;
    }
    let i = b.len() - 1;
    if !b[i].is_ascii_digit() {
        return false;
    }
    let mut j = i;
    while j > 0 && b[j].is_ascii_digit() {
        j -= 1;
    }
    j >= 1 && b[j] == b'v' && b[j - 1] == b'/'
}

/// z.ai GLM Coding Plan Anthropic Messages endpoint (passthrough for Claude Code).
pub const ZAI_ANTHROPIC_BASE: &str = "https://api.z.ai/api/anthropic";

/// If this account is a z.ai OpenAI coding base, return an Anthropic-Messages base rewrite.
pub fn glm_anthropic_messages_base(upstream_base: &str) -> Option<&'static str> {
    let b = upstream_base.to_ascii_lowercase();
    if b.contains("z.ai") && (b.contains("coding/paas") || b.contains("/paas/v")) {
        return Some(ZAI_ANTHROPIC_BASE);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zai_v4_strips_v1_from_chat_path() {
        let u = join_upstream_url_owned(
            "https://api.z.ai/api/coding/paas/v4",
            "/v1/chat/completions",
        );
        assert_eq!(u, "https://api.z.ai/api/coding/paas/v4/chat/completions");
    }

    #[test]
    fn zai_v4_strips_v1_from_messages_path() {
        // Should not be used for messages once rewritten to anthropic base,
        // but join must not produce /v4/v1/messages.
        let u = join_upstream_url_owned(
            "https://api.z.ai/api/coding/paas/v4",
            "/v1/messages",
        );
        assert_eq!(u, "https://api.z.ai/api/coding/paas/v4/messages");
    }

    #[test]
    fn anthropic_official_keeps_v1() {
        let u = join_upstream_url_owned("https://api.anthropic.com", "/v1/messages");
        assert_eq!(u, "https://api.anthropic.com/v1/messages");
    }

    #[test]
    fn zai_anthropic_base_keeps_v1() {
        let u = join_upstream_url_owned(ZAI_ANTHROPIC_BASE, "/v1/messages");
        assert_eq!(u, "https://api.z.ai/api/anthropic/v1/messages");
    }

    #[test]
    fn glm_base_rewrite() {
        assert_eq!(
            glm_anthropic_messages_base("https://api.z.ai/api/coding/paas/v4"),
            Some(ZAI_ANTHROPIC_BASE)
        );
        assert_eq!(
            glm_anthropic_messages_base("https://api.anthropic.com"),
            None
        );
    }
}
