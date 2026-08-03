use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub ready: Arc<AtomicBool>,
}

impl AppState {
    pub async fn new_for_test() -> Self {
        let s = Self {
            ready: Arc::new(AtomicBool::new(true)),
        };
        s
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
}
