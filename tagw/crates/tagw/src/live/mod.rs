//! In-memory live console: broadcast fan-out + ring buffer of recent events.

use std::collections::VecDeque;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};

use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::routing::get;
use axum::{Json, Router};
use futures_util::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::auth::dashboard::AuthUser;
use crate::error::AppError;
use crate::state::AppState;

/// Max events retained for `GET /api/logs/recent`.
pub const RING_CAPACITY: usize = 500;
/// Broadcast channel lag buffer (subscribers that fall behind drop messages).
const BROADCAST_CAPACITY: usize = 256;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LiveEvent {
    pub id: String,
    pub ts: String,
    pub level: String, // info|warn|error
    pub message: String,
    pub request_id: Option<String>,
    pub member_key_id: Option<String>,
    pub model: Option<String>,
}

struct LiveLogHubInner {
    tx: broadcast::Sender<LiveEvent>,
    ring: Mutex<VecDeque<LiveEvent>>,
}

/// Shared live-log hub: publish from the proxy hot path, subscribe via SSE.
#[derive(Clone)]
pub struct LiveLogHub {
    inner: Arc<LiveLogHubInner>,
}

impl LiveLogHub {
    pub fn new() -> Self {
        let (tx, _) = broadcast::channel(BROADCAST_CAPACITY);
        Self {
            inner: Arc::new(LiveLogHubInner {
                tx,
                ring: Mutex::new(VecDeque::with_capacity(RING_CAPACITY)),
            }),
        }
    }

    /// Non-blocking publish: push into ring + broadcast (ignores no-subscriber).
    pub fn publish(&self, event: LiveEvent) {
        {
            let mut ring = self.inner.ring.lock().expect("live ring lock");
            if ring.len() >= RING_CAPACITY {
                ring.pop_front();
            }
            ring.push_back(event.clone());
        }
        // Zero receivers is fine; lagging receivers get Lagged errors on recv.
        let _ = self.inner.tx.send(event);
    }

    /// Snapshot of recent events (oldest → newest).
    pub fn recent(&self, limit: usize) -> Vec<LiveEvent> {
        let ring = self.inner.ring.lock().expect("live ring lock");
        let n = limit.min(ring.len());
        ring.iter()
            .rev()
            .take(n)
            .cloned()
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
        self.inner.tx.subscribe()
    }
}

impl Default for LiveLogHub {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience constructor for proxy completion events.
pub fn request_complete_event(
    request_id: impl Into<String>,
    member_key_id: Option<String>,
    model: Option<String>,
    status: Option<i32>,
    error: Option<&str>,
) -> LiveEvent {
    let status = status.unwrap_or(0);
    let (level, message) = if let Some(err) = error {
        (
            "error".to_string(),
            format!("request failed status={status}: {err}"),
        )
    } else if status >= 500 {
        (
            "error".to_string(),
            format!("request completed status={status}"),
        )
    } else if status >= 400 {
        (
            "warn".to_string(),
            format!("request completed status={status}"),
        )
    } else {
        (
            "info".to_string(),
            format!("request completed status={status}"),
        )
    };
    LiveEvent {
        id: uuid::Uuid::new_v4().to_string(),
        ts: chrono::Utc::now().to_rfc3339(),
        level,
        message,
        request_id: Some(request_id.into()),
        member_key_id,
        model,
    }
}

fn event_to_sse(ev: &LiveEvent) -> Event {
    let data = serde_json::to_string(ev).unwrap_or_else(|_| "{}".into());
    Event::default().data(data).id(ev.id.clone())
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/logs/stream", get(logs_stream))
        .route("/api/logs/recent", get(logs_recent))
}

#[derive(Debug, Deserialize)]
pub struct RecentQuery {
    pub limit: Option<usize>,
}

async fn logs_recent(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<RecentQuery>,
) -> Result<Json<Vec<LiveEvent>>, AppError> {
    let limit = q.limit.unwrap_or(100).clamp(1, RING_CAPACITY);
    Ok(Json(state.live.recent(limit)))
}

async fn logs_stream(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    let rx = state.live.subscribe();
    // Replay recent so a freshly opened console is not empty.
    let replay = state.live.recent(50);
    let replay_stream = stream::iter(
        replay
            .into_iter()
            .map(|ev| Ok::<Event, Infallible>(event_to_sse(&ev))),
    );

    let live_stream = stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    return Some((Ok::<Event, Infallible>(event_to_sse(&ev)), rx));
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Skip gap; client can call /recent if needed.
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    });

    let combined = replay_stream.chain(live_stream);
    Sse::new(combined).keep_alive(KeepAlive::default())
}
