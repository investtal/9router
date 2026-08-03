use anyhow::{anyhow, Context, Result};
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand_core::{OsRng, RngCore};
use rusqlite::params;
use serde::{Deserialize, Serialize};

use crate::db::Db;

/// Authenticated member identity derived from a bearer API key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberContext {
    pub key_id: String,
    pub name: String,
}

/// Row stored for a member API key (never includes plaintext).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemberApiKeyRow {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub key_hash: String,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

/// Public view of a key (prefix only — no hash/plaintext).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemberApiKeyPublic {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

impl From<MemberApiKeyRow> for MemberApiKeyPublic {
    fn from(row: MemberApiKeyRow) -> Self {
        Self {
            id: row.id,
            name: row.name,
            key_prefix: row.key_prefix,
            created_at: row.created_at,
            revoked_at: row.revoked_at,
        }
    }
}

/// Hash a plaintext API key with argon2 (PHC string).
pub fn hash_key(plaintext: &str) -> String {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plaintext.as_bytes(), &salt)
        .expect("argon2 hash should not fail with valid inputs")
        .to_string()
}

/// Verify a plaintext key against an argon2 PHC hash.
pub fn verify_key(plaintext: &str, hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else {
        return false;
    };
    Argon2::default()
        .verify_password(plaintext.as_bytes(), &parsed)
        .is_ok()
}

/// Generate a new secret key: full `sk-...` token and its 8-char lookup prefix.
pub fn generate_key() -> (String, String) {
    let mut bytes = [0u8; 24];
    OsRng.fill_bytes(&mut bytes);
    let full = format!("sk-{}", hex_encode(&bytes));
    let prefix = full.chars().take(8).collect::<String>();
    (full, prefix)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

/// Create a member API key. Returns the DB row and the plaintext secret **once**.
pub fn create_member_key(db: &Db, name: &str) -> Result<(MemberApiKeyRow, String)> {
    let name = name.trim();
    if name.is_empty() {
        return Err(anyhow!("name must not be empty"));
    }

    let (plaintext, key_prefix) = generate_key();
    let key_hash = hash_key(&plaintext);
    let id = uuid::Uuid::new_v4().to_string();
    let created_at = chrono::Utc::now().to_rfc3339();

    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO member_api_keys (id, name, key_prefix, key_hash, created_at, revoked_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
            params![id, name, key_prefix, key_hash, created_at],
        )?;
        Ok(())
    })
    .context("insert member_api_keys")?;

    let row = MemberApiKeyRow {
        id,
        name: name.to_string(),
        key_prefix,
        key_hash,
        created_at,
        revoked_at: None,
    };
    Ok((row, plaintext))
}

/// Soft-revoke a key by id. Returns true if a live key was revoked.
pub fn revoke_member_key(db: &Db, id: &str) -> Result<bool> {
    let now = chrono::Utc::now().to_rfc3339();
    let n = db
        .with_conn(|conn| {
            conn.execute(
                "UPDATE member_api_keys SET revoked_at = ?1
                 WHERE id = ?2 AND revoked_at IS NULL",
                params![now, id],
            )
        })
        .context("revoke member_api_keys")?;
    Ok(n > 0)
}

/// List all member keys (including revoked). Does not expose hashes in public mapping.
pub fn list_member_keys(db: &Db) -> Result<Vec<MemberApiKeyRow>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, name, key_prefix, key_hash, created_at, revoked_at
             FROM member_api_keys
             ORDER BY created_at DESC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(MemberApiKeyRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    key_prefix: r.get(2)?,
                    key_hash: r.get(3)?,
                    created_at: r.get(4)?,
                    revoked_at: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .context("list member_api_keys")
}

/// Load non-revoked keys for the auth cache (id, name, prefix, hash).
pub fn load_active_keys(db: &Db) -> Result<Vec<MemberApiKeyRow>> {
    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT id, name, key_prefix, key_hash, created_at, revoked_at
             FROM member_api_keys
             WHERE revoked_at IS NULL",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(MemberApiKeyRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    key_prefix: r.get(2)?,
                    key_hash: r.get(3)?,
                    created_at: r.get(4)?,
                    revoked_at: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    })
    .context("load active member_api_keys")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_key_has_sk_prefix_and_8_char_lookup() {
        let (full, prefix) = generate_key();
        assert!(full.starts_with("sk-"));
        assert_eq!(prefix.len(), 8);
        assert!(full.starts_with(&prefix));
    }

    #[test]
    fn hash_and_verify_roundtrip() {
        let (full, _) = generate_key();
        let h = hash_key(&full);
        assert!(verify_key(&full, &h));
        assert!(!verify_key("sk-wrong", &h));
    }
}
