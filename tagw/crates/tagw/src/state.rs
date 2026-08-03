use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::db::Db;

#[derive(Clone)]
pub struct AppState {
    pub ready: Arc<AtomicBool>,
    pub db: Db,
}

impl AppState {
    pub fn new(db: Db) -> Self {
        Self {
            ready: Arc::new(AtomicBool::new(true)),
            db,
        }
    }

    pub async fn new_for_test() -> Self {
        let path = std::env::temp_dir().join(format!("tagw-test-{}.db", uuid::Uuid::new_v4()));
        let db = Db::open(&path).expect("open test db");
        db.migrate().expect("migrate test db");
        Self::new(db)
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
}
