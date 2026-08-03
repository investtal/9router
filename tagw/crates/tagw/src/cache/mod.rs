use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};

use crate::auth::member_key::{
    load_active_keys, verify_key, MemberApiKeyRow, MemberContext,
};
use crate::db::Db;
use crate::router::AccountRef;

/// Cached active member key material for hot-path bearer auth.
#[derive(Clone, Debug)]
struct CachedKey {
    id: String,
    name: String,
    key_hash: String,
}

/// Account entry in a routing pool (enablement filtered before pick).
#[derive(Clone, Debug)]
pub struct CachedAccount {
    pub account: AccountRef,
    pub enabled: bool,
}

/// In-memory config/auth cache. Keys indexed by 8-char prefix → candidate hashes.
/// Also holds account pools for the hot-path AccountRouter.
#[derive(Clone)]
pub struct ConfigCache {
    keys_by_prefix: Arc<RwLock<HashMap<String, Vec<CachedKey>>>>,
    /// pool_key → accounts (enabled + disabled). Disabled skipped by [`Self::enabled_accounts`].
    account_pools: Arc<RwLock<HashMap<String, Vec<CachedAccount>>>>,
}

impl ConfigCache {
    pub fn new() -> Self {
        Self {
            keys_by_prefix: Arc::new(RwLock::new(HashMap::new())),
            account_pools: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load active member keys and account pools (api_key + oauth) from SQLite.
    pub fn load(&self, db: &Db) -> Result<()> {
        let rows = load_active_keys(db).context("config cache load keys")?;
        self.replace_keys(rows);
        let api_pools = crate::providers::api_key::load_account_pools(db)
            .context("config cache load api_key account pools")?;
        let oauth_pools = crate::oauth::load_oauth_account_pools(db)
            .context("config cache load oauth account pools")?;
        self.replace_account_pools(crate::oauth::merge_account_pools(api_pools, oauth_pools));
        Ok(())
    }

    /// Reload after mutate (keys or providers). Same as `load`.
    pub fn reload(&self, db: &Db) -> Result<()> {
        self.load(db)
    }

    /// Insert or replace a single active key in-memory (create path).
    /// Keeps auth consistent even if a subsequent full `reload` fails.
    pub fn upsert(&self, row: &MemberApiKeyRow) {
        let mut guard = self
            .keys_by_prefix
            .write()
            .expect("config cache lock poisoned");
        for candidates in guard.values_mut() {
            candidates.retain(|k| k.id != row.id);
        }
        guard.retain(|_, v| !v.is_empty());
        guard
            .entry(row.key_prefix.clone())
            .or_default()
            .push(CachedKey {
                id: row.id.clone(),
                name: row.name.clone(),
                key_hash: row.key_hash.clone(),
            });
    }

    /// Remove a key by id from the in-memory cache (revoke path).
    /// Keeps auth consistent even if a subsequent full `reload` fails.
    pub fn remove_key(&self, id: &str) {
        let mut guard = self
            .keys_by_prefix
            .write()
            .expect("config cache lock poisoned");
        for candidates in guard.values_mut() {
            candidates.retain(|k| k.id != id);
        }
        guard.retain(|_, v| !v.is_empty());
    }

    fn replace_keys(&self, rows: Vec<MemberApiKeyRow>) {
        let mut map: HashMap<String, Vec<CachedKey>> = HashMap::new();
        for row in rows {
            map.entry(row.key_prefix)
                .or_default()
                .push(CachedKey {
                    id: row.id,
                    name: row.name,
                    key_hash: row.key_hash,
                });
        }
        let mut guard = self
            .keys_by_prefix
            .write()
            .expect("config cache lock poisoned");
        *guard = map;
    }

    /// Authenticate a raw bearer token (without the `Bearer ` scheme prefix).
    /// Looks up candidates by the first 8 characters, then verifies argon2.
    pub fn authenticate_bearer(&self, token: &str) -> Option<MemberContext> {
        if token.len() < 8 {
            return None;
        }
        let prefix: String = token.chars().take(8).collect();
        let guard = self.keys_by_prefix.read().ok()?;
        let candidates = guard.get(&prefix)?;
        for c in candidates {
            if verify_key(token, &c.key_hash) {
                return Some(MemberContext {
                    key_id: c.id.clone(),
                    name: c.name.clone(),
                });
            }
        }
        None
    }

    /// Replace the full account pool for `pool_key` (tests + manual inject).
    pub fn set_account_pool(&self, pool_key: impl Into<String>, accounts: Vec<CachedAccount>) {
        let mut guard = self
            .account_pools
            .write()
            .expect("config cache account_pools lock poisoned");
        guard.insert(pool_key.into(), accounts);
    }

    /// Replace all account pools (loaded from DB).
    pub fn replace_account_pools(&self, pools: HashMap<String, Vec<CachedAccount>>) {
        let mut guard = self
            .account_pools
            .write()
            .expect("config cache account_pools lock poisoned");
        *guard = pools;
    }

    /// Clear all account pools (tests).
    pub fn clear_account_pools(&self) {
        let mut guard = self
            .account_pools
            .write()
            .expect("config cache account_pools lock poisoned");
        guard.clear();
    }

    /// Enabled accounts for a pool, in stored order. Disabled entries are skipped.
    pub fn enabled_accounts(&self, pool_key: &str) -> Vec<AccountRef> {
        let guard = self
            .account_pools
            .read()
            .expect("config cache account_pools lock poisoned");
        guard
            .get(pool_key)
            .map(|entries| {
                entries
                    .iter()
                    .filter(|e| e.enabled)
                    .map(|e| e.account.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

impl Default for ConfigCache {
    fn default() -> Self {
        Self::new()
    }
}
