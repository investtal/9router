use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cache::ConfigCache;
use crate::db::Db;
use crate::oauth::new_pending_map;
use crate::oauth::refresh::PendingMap;
use crate::router::AccountRouter;
use crate::usage::{spawn_usage_writer, UsageTx, USAGE_CHANNEL_CAPACITY};

/// Default routing pool key until model→provider mapping lands (Task 7+).
pub const DEFAULT_POOL_KEY: &str = "default";

#[derive(Clone)]
pub struct AppState {
    pub ready: Arc<AtomicBool>,
    pub db: Db,
    pub cache: ConfigCache,
    /// Non-blocking enqueue for request usage / log rows.
    pub usage_tx: UsageTx,
    /// Shared HTTP client for upstream proxy calls (connection pooling).
    pub http_client: reqwest::Client,
    /// Round-robin + fail-over account selection (shared cursors across clones).
    pub account_router: AccountRouter,
    /// Dev fallback upstream base URL (`TAGW_UPSTREAM`) when the account pool is empty.
    pub upstream_base: Option<String>,
    /// Authorization header value for the dev fallback upstream (`TAGW_UPSTREAM_AUTH`).
    pub upstream_auth: Option<String>,
    /// Public base URL for OAuth redirect_uri construction (`TAGW_PUBLIC_BASE`).
    pub public_base: Option<String>,
    /// In-memory PKCE sessions for OAuth start → callback (keyed by state).
    pub oauth_pending: PendingMap,
}

impl AppState {
    pub fn new(db: Db, cache: ConfigCache, usage_tx: UsageTx) -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(true)),
            db,
            cache,
            usage_tx,
            http_client: reqwest::Client::new(),
            account_router: AccountRouter::new(),
            upstream_base: None,
            upstream_auth: None,
            public_base: None,
            oauth_pending: new_pending_map(),
        }
    }

    /// Attach dev fallback upstream config (from env or tests). Used when the
    /// account pool is empty.
    pub fn with_upstream(mut self, base: impl Into<String>, auth: Option<String>) -> Self {
        self.upstream_base = Some(base.into());
        self.upstream_auth = auth;
        self
    }

    /// Public base for OAuth redirect_uri (e.g. `http://127.0.0.1:20128`).
    pub fn with_public_base(mut self, base: impl Into<String>) -> Self {
        self.public_base = Some(base.into());
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
