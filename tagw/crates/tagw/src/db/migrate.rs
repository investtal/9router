use anyhow::{anyhow, Context, Result};
use argon2::password_hash::{PasswordHasher, SaltString};
use argon2::Argon2;
use rand_core::OsRng;
use rusqlite::params;

use super::Db;

const SCHEMA_SQL: &str = include_str!("schema.sql");
const SCHEMA_VERSION: i64 = 1;

pub(super) fn run(db: &Db) -> Result<()> {
    db.with_conn(|conn| {
        conn.execute_batch(SCHEMA_SQL)?;

        let applied: i64 = conn.query_row(
            "SELECT COUNT(*) FROM schema_migrations WHERE version = ?1",
            params![SCHEMA_VERSION],
            |r| r.get(0),
        )?;
        if applied == 0 {
            conn.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                params![SCHEMA_VERSION, chrono::Utc::now().to_rfc3339()],
            )?;
        }
        Ok(())
    })
    .context("apply schema")?;

    seed_admin_if_empty(db).context("seed default admin")?;
    Ok(())
}

fn resolve_admin_password() -> String {
    match std::env::var("TAGW_ADMIN_PASSWORD") {
        Ok(p) if p.is_empty() => {
            tracing::warn!(
                "TAGW_ADMIN_PASSWORD is empty — rejecting empty hash and falling back to default 'admin'"
            );
            "admin".to_string()
        }
        Ok(p) => p,
        Err(_) => "admin".to_string(),
    }
}

fn seed_admin_if_empty(db: &Db) -> Result<()> {
    // Fast path: avoid expensive argon2 when users already exist.
    let count: i64 = db.with_conn(|conn| {
        conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))
    })?;
    if count > 0 {
        return Ok(());
    }

    let password = resolve_admin_password();
    if password == "admin" {
        tracing::warn!(
            "seeding default admin user with password 'admin' — set TAGW_ADMIN_PASSWORD for production"
        );
    }

    let password_hash = hash_password(&password)?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Count + insert in one transaction so concurrent migrate cannot double-seed
    // or hit a spurious UNIQUE failure on username.
    let seeded = db.with_conn(|conn| {
        let tx = conn.unchecked_transaction()?;
        let count: i64 = tx.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0))?;
        let seeded = if count == 0 {
            tx.execute(
                "INSERT INTO users (id, username, password_hash, oidc_sub, role, created_at)
                 VALUES (?1, 'admin', ?2, NULL, 'admin', ?3)",
                params![id, password_hash, now],
            )?;
            true
        } else {
            false
        };
        tx.commit()?;
        Ok(seeded)
    })?;

    if seeded {
        tracing::info!("seeded default admin user");
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|e| anyhow!("argon2 hash failed: {e}"))?
        .to_string();
    Ok(hash)
}
