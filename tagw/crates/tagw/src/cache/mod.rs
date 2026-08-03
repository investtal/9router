use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::auth::member_key::{
    load_active_keys, verify_key, MemberApiKeyRow, MemberContext,
};
use crate::db::Db;
use crate::router::AccountRef;

/// Default TTL for verified member API key memo entries (seconds).
pub const DEFAULT_KEY_CACHE_TTL_SECS: u64 = 300;

/// Env var override for auth memo TTL.
const KEY_CACHE_TTL_ENV: &str = "TAGW_KEY_CACHE_TTL_SECS";

/// Cached active member key material for hot-path bearer auth.
#[derive(Clone, Debug)]
struct CachedKey {
    id: String,
    name: String,
    key_hash: String,
}

/// Successful argon2 verification memo: skip re-hash until TTL.
#[derive(Clone, Debug)]
struct AuthMemoEntry {
    ctx: MemberContext,
    verified_at: Instant,
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
    /// `sha256(token)` hex → verified member context + timestamp.
    auth_memo: Arc<RwLock<HashMap<String, AuthMemoEntry>>>,
    /// pool_key → accounts (enabled + disabled). Disabled skipped by [`Self::enabled_accounts`].
    account_pools: Arc<RwLock<HashMap<String, Vec<CachedAccount>>>>,
    /// Memo TTL (resolved once at construction; env read in [`Self::new`]).
    key_cache_ttl: Duration,
}

impl ConfigCache {
    pub fn new() -> Self {
        Self {
            keys_by_prefix: Arc::new(RwLock::new(HashMap::new())),
            auth_memo: Arc::new(RwLock::new(HashMap::new())),
            account_pools: Arc::new(RwLock::new(HashMap::new())),
            key_cache_ttl: key_cache_ttl_from_env(),
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
        {
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
        // Key material changed — drop all memos (safe; single-key clear would need reverse index).
        self.clear_auth_memo();
    }

    /// Remove a key by id from the in-memory cache (revoke path).
    /// Keeps auth consistent even if a subsequent full `reload` fails.
    pub fn remove_key(&self, id: &str) {
        {
            let mut guard = self
                .keys_by_prefix
                .write()
                .expect("config cache lock poisoned");
            for candidates in guard.values_mut() {
                candidates.retain(|k| k.id != id);
            }
            guard.retain(|_, v| !v.is_empty());
        }
        // Revoke must invalidate any memoized bearer for this key.
        self.clear_auth_memo();
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
        {
            let mut guard = self
                .keys_by_prefix
                .write()
                .expect("config cache lock poisoned");
            *guard = map;
        }
        self.clear_auth_memo();
    }

    /// Drop the entire verified-token memo (reload / revoke / upsert).
    pub fn clear_auth_memo(&self) {
        let mut guard = self
            .auth_memo
            .write()
            .expect("config cache auth_memo lock poisoned");
        guard.clear();
    }

    /// Number of entries currently in the auth memo (tests / diagnostics).
    #[cfg(test)]
    pub fn auth_memo_len(&self) -> usize {
        self.auth_memo
            .read()
            .expect("config cache auth_memo lock poisoned")
            .len()
    }

    /// Authenticate a raw bearer token (without the `Bearer ` scheme prefix).
    ///
    /// Fast path: `sha256(token)` hit within TTL skips argon2.
    /// Slow path: prefix lookup + argon2 verify, then memoize success.
    pub fn authenticate_bearer(&self, token: &str) -> Option<MemberContext> {
        if token.len() < 8 {
            return None;
        }

        let token_digest = sha256_hex(token);

        // Fast path: memo hit.
        if let Some(ctx) = self.auth_memo_get(&token_digest) {
            return Some(ctx);
        }

        let prefix: String = token.chars().take(8).collect();
        let guard = self.keys_by_prefix.read().ok()?;
        let candidates = guard.get(&prefix)?;
        for c in candidates {
            if verify_key(token, &c.key_hash) {
                let ctx = MemberContext {
                    key_id: c.id.clone(),
                    name: c.name.clone(),
                };
                drop(guard);
                self.auth_memo_insert(token_digest, ctx.clone());
                return Some(ctx);
            }
        }
        None
    }

    fn auth_memo_get(&self, digest: &str) -> Option<MemberContext> {
        let guard = self.auth_memo.read().ok()?;
        let entry = guard.get(digest)?;
        if entry.verified_at.elapsed() < self.key_cache_ttl {
            Some(entry.ctx.clone())
        } else {
            None
        }
    }

    fn auth_memo_insert(&self, digest: String, ctx: MemberContext) {
        let mut guard = self
            .auth_memo
            .write()
            .expect("config cache auth_memo lock poisoned");
        // Opportunistic prune of expired entries when map grows large.
        if guard.len() > 10_000 {
            let ttl = self.key_cache_ttl;
            guard.retain(|_, e| e.verified_at.elapsed() < ttl);
        }
        guard.insert(
            digest,
            AuthMemoEntry {
                ctx,
                verified_at: Instant::now(),
            },
        );
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

fn key_cache_ttl_from_env() -> Duration {
    let secs = std::env::var(KEY_CACHE_TTL_ENV)
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(DEFAULT_KEY_CACHE_TTL_SECS);
    Duration::from_secs(secs)
}

fn sha256_hex(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    let dig = hasher.finalize();
    let mut out = String::with_capacity(dig.len() * 2);
    for b in dig {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::member_key::{create_member_key, hash_key};
    use crate::db::Db;

    #[tokio::test]
    async fn authenticate_twice_succeeds_and_memoizes() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("gateway.db")).unwrap();
        db.migrate().unwrap();

        let (row, plaintext) = create_member_key(&db, "memo-user").unwrap();
        let cache = ConfigCache::new();
        cache.load(&db).unwrap();

        assert_eq!(cache.auth_memo_len(), 0);
        let a = cache
            .authenticate_bearer(&plaintext)
            .expect("first auth");
        assert_eq!(a.key_id, row.id);
        assert_eq!(cache.auth_memo_len(), 1, "successful verify must memoize");

        let b = cache
            .authenticate_bearer(&plaintext)
            .expect("second auth (memo hit)");
        assert_eq!(b, a);
        assert_eq!(cache.auth_memo_len(), 1);
    }

    #[tokio::test]
    async fn remove_key_clears_auth_memo() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("gateway.db")).unwrap();
        db.migrate().unwrap();

        let (row, plaintext) = create_member_key(&db, "revoke-memo").unwrap();
        let cache = ConfigCache::new();
        cache.load(&db).unwrap();

        assert!(cache.authenticate_bearer(&plaintext).is_some());
        assert_eq!(cache.auth_memo_len(), 1);

        cache.remove_key(&row.id);
        assert_eq!(cache.auth_memo_len(), 0, "remove_key must clear memo");
        assert!(
            cache.authenticate_bearer(&plaintext).is_none(),
            "after remove_key token must not authenticate"
        );
    }

    #[tokio::test]
    async fn reload_clears_auth_memo() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("gateway.db")).unwrap();
        db.migrate().unwrap();

        let (_row, plaintext) = create_member_key(&db, "reload-memo").unwrap();
        let cache = ConfigCache::new();
        cache.load(&db).unwrap();
        assert!(cache.authenticate_bearer(&plaintext).is_some());
        assert_eq!(cache.auth_memo_len(), 1);

        cache.reload(&db).unwrap();
        assert_eq!(cache.auth_memo_len(), 0, "reload must clear memo");
        // Key still active — re-auth works and re-memoizes.
        assert!(cache.authenticate_bearer(&plaintext).is_some());
        assert_eq!(cache.auth_memo_len(), 1);
    }

    #[tokio::test]
    async fn upsert_clears_auth_memo() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(dir.path().join("gateway.db")).unwrap();
        db.migrate().unwrap();

        let (row, plaintext) = create_member_key(&db, "upsert-memo").unwrap();
        let cache = ConfigCache::new();
        cache.load(&db).unwrap();
        assert!(cache.authenticate_bearer(&plaintext).is_some());
        assert_eq!(cache.auth_memo_len(), 1);

        // Upsert same row (simulates create-path re-inject).
        cache.upsert(&row);
        assert_eq!(cache.auth_memo_len(), 0, "upsert must clear memo");
        assert!(cache.authenticate_bearer(&plaintext).is_some());
    }

    #[test]
    fn memo_skips_argon2_after_first_verify() {
        // Inject a key with a real argon2 hash, auth once, then swap the stored
        // hash to garbage. Memo must still return Some (argon2 not re-run).
        let cache = ConfigCache::new();
        let plaintext = "sk-testmemoskip0000000000000001";
        let prefix: String = plaintext.chars().take(8).collect();
        let good_hash = hash_key(plaintext);
        {
            let mut guard = cache.keys_by_prefix.write().unwrap();
            guard.insert(
                prefix.clone(),
                vec![CachedKey {
                    id: "k1".into(),
                    name: "memo-skip".into(),
                    key_hash: good_hash,
                }],
            );
        }
        let ctx = cache.authenticate_bearer(plaintext).expect("first verify");
        assert_eq!(ctx.key_id, "k1");
        assert_eq!(cache.auth_memo_len(), 1);

        // Corrupt stored hash — only memo can make second call succeed.
        {
            let mut guard = cache.keys_by_prefix.write().unwrap();
            guard.get_mut(&prefix).unwrap()[0].key_hash = "not-a-valid-phc".into();
        }
        let again = cache
            .authenticate_bearer(plaintext)
            .expect("memo must skip argon2");
        assert_eq!(again.key_id, "k1");

        // After clear, verify must fail on garbage hash.
        cache.clear_auth_memo();
        assert!(cache.authenticate_bearer(plaintext).is_none());
    }

    #[test]
    fn sha256_hex_stable() {
        assert_eq!(
            sha256_hex("hello"),
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
        );
    }
}
