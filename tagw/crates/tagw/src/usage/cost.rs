//! Model pricing tables and USD cost estimation for usage events.
//!
//! Rates are **ballpark public list prices** (USD per 1M tokens), not contractual.
//! Unknown models return `0.0` so cost never blocks the proxy path.

/// Pricing for one model family (USD per 1_000_000 tokens).
#[derive(Clone, Copy, Debug)]
struct ModelRates {
    /// Substring / prefix match key (lowercased). Longer keys preferred.
    key: &'static str,
    input_per_m: f64,
    output_per_m: f64,
    /// Cached / cache-read input rate. When `None`, cached tokens use `input_per_m`.
    cached_input_per_m: Option<f64>,
}

/// Static pricing table. Order does not matter — longest matching key wins.
const MODEL_RATES: &[ModelRates] = &[
    // ── OpenAI-ish ──────────────────────────────────────────────────────────
    ModelRates {
        key: "gpt-4o-mini",
        input_per_m: 0.15,
        output_per_m: 0.60,
        cached_input_per_m: Some(0.075),
    },
    ModelRates {
        key: "gpt-4o",
        input_per_m: 2.50,
        output_per_m: 10.00,
        cached_input_per_m: Some(1.25),
    },
    ModelRates {
        key: "gpt-4.1",
        input_per_m: 2.00,
        output_per_m: 8.00,
        cached_input_per_m: Some(0.50),
    },
    ModelRates {
        key: "o3-mini",
        input_per_m: 1.10,
        output_per_m: 4.40,
        cached_input_per_m: Some(0.55),
    },
    ModelRates {
        key: "o4-mini",
        input_per_m: 1.10,
        output_per_m: 4.40,
        cached_input_per_m: Some(0.275),
    },
    // ── Anthropic ───────────────────────────────────────────────────────────
    ModelRates {
        key: "claude-sonnet-4",
        input_per_m: 3.00,
        output_per_m: 15.00,
        cached_input_per_m: Some(0.30),
    },
    ModelRates {
        key: "claude-opus-4",
        input_per_m: 15.00,
        output_per_m: 75.00,
        cached_input_per_m: Some(1.50),
    },
    ModelRates {
        key: "claude-haiku",
        input_per_m: 0.80,
        output_per_m: 4.00,
        cached_input_per_m: Some(0.08),
    },
    // ── Cheap coding / OpenAI-compat ────────────────────────────────────────
    ModelRates {
        key: "deepseek-chat",
        input_per_m: 0.27,
        output_per_m: 1.10,
        cached_input_per_m: Some(0.07),
    },
    ModelRates {
        key: "deepseek",
        input_per_m: 0.27,
        output_per_m: 1.10,
        cached_input_per_m: Some(0.07),
    },
    ModelRates {
        key: "glm-4",
        input_per_m: 0.10,
        output_per_m: 0.10,
        cached_input_per_m: None,
    },
    ModelRates {
        key: "glm",
        input_per_m: 0.10,
        output_per_m: 0.10,
        cached_input_per_m: None,
    },
    ModelRates {
        key: "minimax",
        input_per_m: 0.20,
        output_per_m: 1.10,
        cached_input_per_m: None,
    },
];

/// Find the best (longest key) matching rate for `model` (case-insensitive contains).
fn lookup_rates(model: &str) -> Option<&'static ModelRates> {
    let lower = model.to_ascii_lowercase();
    let mut best: Option<&'static ModelRates> = None;
    for rate in MODEL_RATES {
        if lower.contains(rate.key) {
            match best {
                None => best = Some(rate),
                Some(prev) if rate.key.len() > prev.key.len() => best = Some(rate),
                _ => {}
            }
        }
    }
    best
}

/// Estimate request cost in USD from model + token counts.
///
/// - `prompt_tokens` / `completion_tokens` / `cached_tokens` are raw counts.
/// - When `cached_tokens > 0` and the model has a cached rate, non-cached input
///   is `prompt - cached` at full input rate and cached at the discounted rate.
/// - Unknown / missing model → `0.0`.
pub fn estimate_cost(
    model: Option<&str>,
    prompt_tokens: i64,
    completion_tokens: i64,
    cached_tokens: i64,
) -> f64 {
    let Some(model) = model.map(str::trim).filter(|s| !s.is_empty()) else {
        return 0.0;
    };
    let Some(rates) = lookup_rates(model) else {
        return 0.0;
    };

    let prompt = prompt_tokens.max(0) as f64;
    let completion = completion_tokens.max(0) as f64;
    let cached = (cached_tokens.max(0) as f64).min(prompt);

    let (uncached_prompt, cached_prompt) = if rates.cached_input_per_m.is_some() && cached > 0.0 {
        (prompt - cached, cached)
    } else {
        (prompt, 0.0)
    };

    let cached_rate = rates.cached_input_per_m.unwrap_or(rates.input_per_m);
    let input_cost = (uncached_prompt * rates.input_per_m + cached_prompt * cached_rate) / 1_000_000.0;
    let output_cost = (completion * rates.output_per_m) / 1_000_000.0;
    input_cost + output_cost
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_model_nonzero() {
        let c = estimate_cost(Some("gpt-4o"), 1_000_000, 1_000_000, 0);
        assert!(c > 0.0, "gpt-4o must have non-zero cost, got {c}");
        // 2.50 + 10.00 per 1M each = 12.50
        assert!((c - 12.50).abs() < 1e-9, "got {c}");
    }

    #[test]
    fn unknown_model_zero() {
        assert_eq!(estimate_cost(Some("totally-unknown-xyz"), 100, 100, 0), 0.0);
        assert_eq!(estimate_cost(None, 100, 100, 0), 0.0);
        assert_eq!(estimate_cost(Some(""), 100, 100, 0), 0.0);
    }

    #[test]
    fn longest_match_wins() {
        // gpt-4o-mini must not use gpt-4o rates.
        let mini = estimate_cost(Some("gpt-4o-mini-2024-07-18"), 1_000_000, 0, 0);
        let full = estimate_cost(Some("gpt-4o-2024-08-06"), 1_000_000, 0, 0);
        assert!((mini - 0.15).abs() < 1e-9, "mini got {mini}");
        assert!((full - 2.50).abs() < 1e-9, "full got {full}");
        assert!(mini < full);
    }

    #[test]
    fn case_insensitive_contains() {
        let c = estimate_cost(Some("GPT-4O"), 1_000_000, 0, 0);
        assert!((c - 2.50).abs() < 1e-9);
        let ds = estimate_cost(Some("deepseek-chat-v3"), 1_000_000, 0, 0);
        assert!((ds - 0.27).abs() < 1e-9);
    }

    #[test]
    fn cached_discount_when_modeled() {
        // Claude sonnet: input 3.00, cached 0.30 per 1M.
        // 500k uncached + 500k cached → 1.5 + 0.15 = 1.65
        let with_cache = estimate_cost(Some("claude-sonnet-4-20250514"), 1_000_000, 0, 500_000);
        assert!(
            (with_cache - 1.65).abs() < 1e-9,
            "cached discount expected 1.65, got {with_cache}"
        );
        let no_cache = estimate_cost(Some("claude-sonnet-4-20250514"), 1_000_000, 0, 0);
        assert!((no_cache - 3.00).abs() < 1e-9);
        assert!(with_cache < no_cache);
    }

    #[test]
    fn anthropic_and_cheap_coding_nonzero() {
        assert!(estimate_cost(Some("claude-opus-4"), 1000, 1000, 0) > 0.0);
        assert!(estimate_cost(Some("claude-haiku-3.5"), 1000, 1000, 0) > 0.0);
        assert!(estimate_cost(Some("glm-4-flash"), 1000, 1000, 0) > 0.0);
        assert!(estimate_cost(Some("minimax-text-01"), 1000, 1000, 0) > 0.0);
        assert!(estimate_cost(Some("o3-mini"), 1000, 1000, 0) > 0.0);
        assert!(estimate_cost(Some("o4-mini"), 1000, 1000, 0) > 0.0);
        assert!(estimate_cost(Some("gpt-4.1"), 1000, 1000, 0) > 0.0);
    }
}
