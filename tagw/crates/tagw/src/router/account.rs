//! Round-robin account selection with fail-over status helpers.
//!
//! Cursor state is per `pool_key` (e.g. provider id or `"default"`). Callers
//! filter disabled accounts before [`AccountRouter::pick`] (or pass only enabled
//! slices from [`crate::cache::ConfigCache::enabled_accounts`]).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Maximum upstream attempts per client request (initial pick + fail-overs).
pub const MAX_FAILOVER_ATTEMPTS: usize = 3;

/// Resolved upstream account ready for a proxy hop.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRef {
    pub account_id: String,
    pub provider_id: String,
    pub upstream_base: String,
    /// Authorization header value, e.g. `"Bearer sk-..."` or a raw token.
    pub auth_header: String,
}

/// Round-robin router. Thread-safe cursors keyed by pool; shared across clones.
#[derive(Clone, Debug, Default)]
pub struct AccountRouter {
    cursors: Arc<Mutex<HashMap<String, usize>>>,
}

impl AccountRouter {
    pub fn new() -> Self {
        Self {
            cursors: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Pick the next account in the pool using round-robin.
    ///
    /// Advances the pool cursor on every successful pick. Returns `None` when
    /// `accounts` is empty. Does not interpret enablement — pass a pre-filtered
    /// slice of enabled accounts.
    pub fn pick(&self, pool_key: &str, accounts: &[AccountRef]) -> Option<AccountRef> {
        if accounts.is_empty() {
            return None;
        }
        let mut guard = self
            .cursors
            .lock()
            .expect("account router cursor lock poisoned");
        let cursor = guard.entry(pool_key.to_string()).or_insert(0);
        let idx = *cursor % accounts.len();
        *cursor = cursor.wrapping_add(1);
        Some(accounts[idx].clone())
    }

    /// Status codes that trigger fail-over before any response byte is forwarded.
    pub fn should_failover(status: u16) -> bool {
        matches!(status, 429 | 500 | 502 | 503 | 504)
    }

    /// Current cursor for a pool (test / diagnostics). Missing pools report `0`.
    #[cfg(test)]
    pub fn cursor(&self, pool_key: &str) -> usize {
        self.cursors
            .lock()
            .expect("account router cursor lock poisoned")
            .get(pool_key)
            .copied()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(id: &str) -> AccountRef {
        AccountRef {
            account_id: id.into(),
            provider_id: "prov".into(),
            upstream_base: format!("http://{id}.example"),
            auth_header: format!("Bearer {id}"),
        }
    }

    #[test]
    fn pick_round_robin_advances_cursor() {
        let router = AccountRouter::new();
        let pool = vec![acct("a"), acct("b"), acct("c")];
        assert_eq!(router.pick("p", &pool).unwrap().account_id, "a");
        assert_eq!(router.pick("p", &pool).unwrap().account_id, "b");
        assert_eq!(router.pick("p", &pool).unwrap().account_id, "c");
        assert_eq!(router.pick("p", &pool).unwrap().account_id, "a");
        assert_eq!(router.cursor("p"), 4);
    }

    #[test]
    fn pick_empty_returns_none() {
        let router = AccountRouter::new();
        assert!(router.pick("p", &[]).is_none());
        assert_eq!(router.cursor("p"), 0);
    }

    #[test]
    fn pick_cursors_are_per_pool() {
        let router = AccountRouter::new();
        let pool = vec![acct("a"), acct("b")];
        assert_eq!(router.pick("pool-1", &pool).unwrap().account_id, "a");
        assert_eq!(router.pick("pool-2", &pool).unwrap().account_id, "a");
        assert_eq!(router.pick("pool-1", &pool).unwrap().account_id, "b");
        assert_eq!(router.pick("pool-2", &pool).unwrap().account_id, "b");
    }

    #[test]
    fn clones_share_cursor_state() {
        let router = AccountRouter::new();
        let clone = router.clone();
        let pool = vec![acct("a"), acct("b")];
        assert_eq!(router.pick("p", &pool).unwrap().account_id, "a");
        assert_eq!(clone.pick("p", &pool).unwrap().account_id, "b");
    }

    #[test]
    fn should_failover_statuses() {
        for s in [429u16, 500, 502, 503, 504] {
            assert!(AccountRouter::should_failover(s), "status {s}");
        }
        for s in [200u16, 201, 400, 401, 403, 404, 408, 501, 505] {
            assert!(!AccountRouter::should_failover(s), "status {s}");
        }
    }

    #[test]
    fn max_failover_attempts_is_three() {
        assert_eq!(MAX_FAILOVER_ATTEMPTS, 3);
    }
}
