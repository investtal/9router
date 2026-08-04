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
    /// Client request body (truncated UTF-8 JSON), for request detail UI.
    pub request_body: Option<String>,
    /// Upstream response body / SSE (truncated UTF-8), for request detail UI.
    pub response_body: Option<String>,
}

/// Default max **bytes** stored per body field.
///
/// Sized for analytics (prompts + first SSE chunks) without multi‑MB SQLite rows.
/// Override with `TAGW_BODY_MAX_BYTES` (clamp 4 KiB … 2 MiB).
pub const BODY_STORE_MAX_BYTES_DEFAULT: usize = 64 * 1024;

/// Alias kept for call sites; means max **bytes**.
pub const BODY_STORE_MAX_CHARS: usize = BODY_STORE_MAX_BYTES_DEFAULT;

/// Resolved cap (env once). Cheap: `OnceLock`.
pub fn body_store_max_bytes() -> usize {
    use std::sync::OnceLock;
    static MAX: OnceLock<usize> = OnceLock::new();
    *MAX.get_or_init(|| {
        let raw = std::env::var("TAGW_BODY_MAX_BYTES")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(BODY_STORE_MAX_BYTES_DEFAULT);
        raw.clamp(4 * 1024, 2 * 1024 * 1024)
    })
}

/// Zero-allocation-friendly response body accumulator for the stream hot path.
///
/// - Appends raw bytes (no UTF-8 check per chunk)
/// - Stops after cap (further `push` is a single bool check)
/// - Converts to String **once** at EOS for the async usage writer
#[derive(Debug, Default)]
pub struct BodyCapture {
    buf: Vec<u8>,
    max: usize,
    truncated: bool,
}

impl BodyCapture {
    pub fn with_max(max: usize) -> Self {
        Self {
            buf: Vec::with_capacity(max.min(8 * 1024)),
            max,
            truncated: false,
        }
    }

    #[inline]
    pub fn push(&mut self, chunk: &[u8]) {
        if self.truncated || chunk.is_empty() {
            return;
        }
        let room = self.max.saturating_sub(self.buf.len());
        if room == 0 {
            self.truncated = true;
            return;
        }
        if chunk.len() <= room {
            self.buf.extend_from_slice(chunk);
            if self.buf.len() >= self.max {
                self.truncated = true;
            }
        } else {
            self.buf.extend_from_slice(&chunk[..room]);
            self.truncated = true;
        }
    }

    pub fn into_stored(self) -> Option<String> {
        if self.buf.is_empty() {
            return None;
        }
        let total_hint = self.buf.len();
        let s = String::from_utf8_lossy(&self.buf);
        if self.truncated {
            Some(format!(
                "{s}\n…[truncated at {} bytes for storage]",
                self.max.max(total_hint)
            ))
        } else {
            Some(s.into_owned())
        }
    }
}

/// Truncate a UTF-8 body for storage (byte-capped; no O(n) `chars().count()`).
pub fn truncate_body_for_storage(raw: &str, max_bytes: usize) -> String {
    if raw.len() <= max_bytes {
        return raw.to_string();
    }
    // Walk back to a char boundary.
    let mut end = max_bytes.min(raw.len());
    while end > 0 && !raw.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n…[truncated {} bytes total]",
        &raw[..end],
        raw.len()
    )
}

/// Best-effort UTF-8 lossy string from request bytes, truncated (single pass).
///
/// Request body is already buffered for fail-over; this only runs **once** per
/// request (not on the stream hot path).
pub fn body_bytes_for_storage(bytes: &[u8], max_bytes: usize) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    if bytes.len() <= max_bytes {
        return Some(String::from_utf8_lossy(bytes).into_owned());
    }
    let mut end = max_bytes;
    // Find a valid UTF-8 boundary by walking back from max_bytes.
    while end > 0 && (bytes[end] & 0b1100_0000) == 0b1000_0000 {
        end -= 1;
    }
    let s = String::from_utf8_lossy(&bytes[..end]);
    Some(format!(
        "{s}\n…[truncated {} bytes total]",
        bytes.len()
    ))
}

/// Sender half of the usage write queue (clone onto request state).
pub type UsageTx = tokio::sync::mpsc::Sender<UsageEvent>;
