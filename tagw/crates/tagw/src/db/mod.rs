mod migrate;

use std::path::Path;
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use rusqlite::Connection;

/// SQLite database handle (single writer via mutex, WAL mode).
#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    /// Open (or create) a SQLite database at `path`. Enables WAL and foreign keys.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create data dir {}", parent.display()))?;
            }
        }
        let conn = Connection::open(path)
            .with_context(|| format!("open sqlite database {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .context("set journal_mode=WAL")?;
        conn.pragma_update(None, "foreign_keys", true)
            .context("enable foreign_keys")?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Run a closure with exclusive access to the connection.
    pub fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> rusqlite::Result<T>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| anyhow!("db connection lock poisoned: {e}"))?;
        f(&conn).map_err(Into::into)
    }

    /// Apply schema migrations and seed default admin if needed.
    pub fn migrate(&self) -> Result<()> {
        migrate::run(self)
    }
}
