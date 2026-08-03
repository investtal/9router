//! Async non-blocking usage / request-log recording.
//!
//! Producers enqueue [`UsageEvent`]s on a bounded mpsc channel; a dedicated
//! writer task batches inserts into SQLite so the proxy hot path never awaits
//! disk I/O.

mod cost;
pub mod query;
mod writer;

pub use cost::estimate_cost;
pub use writer::spawn_usage_writer;

/// Default capacity for the usage channel (non-blocking buffer for producers).
pub const USAGE_CHANNEL_CAPACITY: usize = 1024;

/// One proxied request's usage / log row.
#[derive(Clone, Debug)]
pub struct UsageEvent {
    pub id: String,
    /// RFC3339 timestamp.
    pub created_at: String,
    pub member_key_id: Option<String>,
    pub provider_id: Option<String>,
    pub account_id: Option<String>,
    pub model: Option<String>,
    pub tool: Option<String>,
    pub status: Option<i32>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub cost_est: f64,
    pub latency_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub usage_incomplete: bool,
    pub error: Option<String>,
}

/// Sender half of the usage write queue (clone onto request state).
pub type UsageTx = tokio::sync::mpsc::Sender<UsageEvent>;
