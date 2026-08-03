use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};

use crate::auth::member_key::{
    load_active_keys, verify_key, MemberApiKeyRow, MemberContext,
};
use crate::db::Db;

/// Cached active member key material for hot-path bearer auth.
#[derive(Clone, Debug)]
struct CachedKey {
    id: String,
    name: String,
    key_hash: String,
}

/// In-memory config/auth cache. Keys indexed by 8-char prefix → candidate hashes.
#[derive(Clone)]
pub struct ConfigCache {
    keys_by_prefix: Arc<RwLock<HashMap<String, Vec<CachedKey>>>>,
}

impl ConfigCache {
    pub fn new() -> Self {
        Self {
            keys_by_prefix: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Load active (non-revoked) member keys from SQLite into the cache.
    pub fn load(&self, db: &Db) -> Result<()> {
        let rows = load_active_keys(db).context("config cache load keys")?;
        self.replace_keys(rows);
        Ok(())
    }

    /// Reload after mutate (create/revoke). Same as `load`.
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
}

impl Default for ConfigCache {
    fn default() -> Self {
        Self::new()
    }
}
