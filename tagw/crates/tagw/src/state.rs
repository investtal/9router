use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cache::ConfigCache;
use crate::db::Db;
use crate::usage::{spawn_usage_writer, UsageTx, USAGE_CHANNEL_CAPACITY};

#[derive(Clone)]
pub struct AppState {
    pub ready: Arc<AtomicBool>,
    pub db: Db,
    pub cache: ConfigCache,
    /// Non-blocking enqueue for request usage / log rows.
    pub usage_tx: UsageTx,
    /// Shared HTTP client for upstream proxy calls (connection pooling).
    pub http_client: reqwest::Client,
    /// Temporary single upstream base URL (`TAGW_UPSTREAM`). Task 6 replaces with AccountRouter.
    pub upstream_base: Option<String>,
    /// Authorization header value for the temporary upstream (`TAGW_UPSTREAM_AUTH`).
    pub upstream_auth: Option<String>,
}

impl AppState {
    pub fn new(db: Db, cache: ConfigCache, usage_tx: UsageTx) -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(true)),
            db,
            cache,
            usage_tx,
            http_client: reqwest::Client::new(),
            upstream_base: None,
            upstream_auth: None,
        }
    }

    /// Attach temporary upstream config (from env or tests).
    pub fn with_upstream(mut self, base: impl Into<String>, auth: Option<String>) -> Self {
        self.upstream_base = Some(base.into());
        self.upstream_auth = auth;
        self
    }

    /// Open a temp DB, migrate, spawn usage writer, load config cache (for integration tests).
    pub async fn new_for_test() -> Self {
        let path = std::env::temp_dir().join(format!("tagw-test-{}.db", uuid::Uuid::new_v4()));
        let db = Db::open(&path).expect("open test db");
        db.migrate().expect("migrate test db");
        let cache = ConfigCache::new();
        cache.load(&db).expect("load config cache");
        let (usage_tx, usage_rx) = tokio::sync::mpsc::channel(USAGE_CHANNEL_CAPACITY);
        // Keep the writer alive for the lifetime of the test process (detached).
        let _writer = spawn_usage_writer(db.clone(), usage_rx);
        Self::new(db, cache, usage_tx)
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
}
