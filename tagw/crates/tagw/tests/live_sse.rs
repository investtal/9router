//! Live console: ring buffer + SSE stream delivery.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use futures_util::StreamExt;
use serde_json::Value;
use tagw::app::build_app;
use tagw::live::{LiveEvent, LiveLogHub, RING_CAPACITY};
use tagw::state::AppState;
use tower::ServiceExt;

fn sample_event(id: &str, message: &str) -> LiveEvent {
    LiveEvent {
        id: id.into(),
        ts: "2026-08-03T12:00:00Z".into(),
        level: "info".into(),
        message: message.into(),
        request_id: Some("req-1".into()),
        member_key_id: Some("key-1".into()),
        model: Some("gpt-4o".into()),
    }
}

#[test]
fn hub_ring_caps_at_500_and_recent_order() {
    let hub = LiveLogHub::new();
    for i in 0..RING_CAPACITY + 10 {
        hub.publish(sample_event(&format!("e{i}"), &format!("msg-{i}")));
    }
    let recent = hub.recent(RING_CAPACITY);
    assert_eq!(recent.len(), RING_CAPACITY);
    // Oldest retained is e10; newest is e{RING_CAPACITY+9}
    assert_eq!(recent.first().unwrap().id, "e10");
    assert_eq!(
        recent.last().unwrap().id,
        format!("e{}", RING_CAPACITY + 9)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recent_endpoint_returns_published_events() {
    let state = AppState::new_for_test().await;
    state.live.publish(sample_event("a", "hello"));
    state.live.publish(sample_event("b", "world"));

    let app = build_app(state.clone());
    let cookie = state.test_session_cookie("admin");

    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/logs/recent?limit=10")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let events: Vec<Value> = serde_json::from_slice(&body).unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["id"], "a");
    assert_eq!(events[1]["message"], "world");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sse_stream_delivers_published_json_event() {
    let state = AppState::new_for_test().await;
    let app = build_app(state.clone());
    let cookie = state.test_session_cookie("admin");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Give server a moment to accept.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let client = reqwest::Client::new();
    let url = format!("http://{addr}/api/logs/stream");
    let response = client
        .get(&url)
        .header("cookie", &cookie)
        .send()
        .await
        .expect("sse connect");
    assert_eq!(response.status(), StatusCode::OK);
    let ct = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        ct.starts_with("text/event-stream"),
        "content-type={ct}"
    );

    // Publish after client is connected so the event is not only in replay.
    state.live.publish(sample_event("live-1", "from-proxy"));

    // Read stream bytes until we see our event (with timeout).
    let mut body = response.bytes_stream();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut buf = String::new();
    loop {
        if tokio::time::Instant::now() > deadline {
            panic!("timeout waiting for SSE event; buf={buf}");
        }
        let next = tokio::time::timeout(Duration::from_millis(500), body.next()).await;
        match next {
            Ok(Some(Ok(chunk))) => {
                buf.push_str(&String::from_utf8_lossy(&chunk));
                if buf.contains("live-1") && buf.contains("from-proxy") {
                    // Ensure data line is JSON
                    assert!(buf.contains("data:"));
                    return;
                }
            }
            Ok(Some(Err(e))) => panic!("stream error: {e}"),
            Ok(None) => panic!("stream closed early; buf={buf}"),
            Err(_) => continue, // timeout on chunk — keep waiting until deadline
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn logs_require_auth() {
    let state = AppState::new_for_test().await;
    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/logs/recent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
