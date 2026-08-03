use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::cache::ConfigCache;
use crate::db::Db;

#[derive(Clone)]
pub struct AppState {
    pub ready: Arc<AtomicBool>,
    pub db: Db,
    pub cache: ConfigCache,
}

impl AppState {
    pub fn new(db: Db, cache: ConfigCache) -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(true)),
            db,
            cache,
        }
    }

    /// Open a temp DB, migrate, and load an empty config cache (for integration tests).
    pub async fn new_for_test() -> Self {
        let path = std::env::temp_dir().join(format!("tagw-test-{}.db", uuid::Uuid::new_v4()));
        let db = Db::open(&path).expect("open test db");
        db.migrate().expect("migrate test db");
        let cache = ConfigCache::new();
        cache.load(&db).expect("load config cache");
        Self::new(db, cache)
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
}
