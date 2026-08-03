//! Integration: OpenAI-compatible streaming proxy with zero full-body buffer.
//!
//! Mock upstream emits 3 SSE chunks with delays. Gateway must stream them through
//! (using `bytes_stream()`, never `response.bytes().await` on the stream path).

use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use bytes::Bytes;
use futures_util::StreamExt;
use tagw::app::build_app;
use tagw::auth::member_key::create_member_key;
use tagw::state::AppState;
use tower::ServiceExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Spawn a real streaming mock that delays between SSE chunks (wiremock alone
/// cannot interleave delays mid-body).
async fn spawn_delayed_sse_upstream() -> String {
    async fn handler(headers: HeaderMap, _body: Bytes) -> impl IntoResponse {
        // Assert upstream auth from gateway config (not the member key).
        let auth = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(auth, "Bearer upstream-secret-key");

        let stream = futures_util::stream::unfold(0u8, |i| async move {
            if i >= 3 {
                return None;
            }
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            let chunk = match i {
                0 => Bytes::from(
                    "data: {\"id\":\"chunk-1\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
                ),
                1 => Bytes::from(
                    "data: {\"id\":\"chunk-2\",\"object\":\"chat.completion.chunk\",\"choices\":[{\"delta\":{\"content\":\" world\"}}]}\n\n",
                ),
                _ => Bytes::from(
                    "data: {\"id\":\"chunk-3\",\"object\":\"chat.completion.chunk\",\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n",
                ),
            };
            Some((Ok::<Bytes, std::io::Error>(chunk), i + 1))
        });

        (
            StatusCode::OK,
            [
                (axum::http::header::CONTENT_TYPE, "text/event-stream"),
                (axum::http::header::CACHE_CONTROL, "no-cache"),
            ],
            Body::from_stream(stream),
        )
    }

    let app = Router::new().route("/v1/chat/completions", post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    format!("http://{addr}")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_completions_streams_chunks_without_buffering_all() {
    let upstream = spawn_delayed_sse_upstream().await;

    let state = AppState::new_for_test()
        .await
        .with_upstream(
            upstream,
            Some("Bearer upstream-secret-key".into()),
        );

    // Create a member API key and load into cache (new_for_test already loaded empty).
    let (row, plaintext) = create_member_key(&state.db, "stream-tester").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state);

    let req = Request::builder()
        .method("POST")
        .uri("/v1/chat/completions")
        .header("authorization", format!("Bearer {plaintext}"))
        .header("content-type", "application/json")
        .body(Body::from(
            r#"{"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
        ))
        .unwrap();

    let res = app.oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Stream body after headers. Inter-chunk delays from the mock (2×40ms) must be
    // observed *while reading the body*. If the gateway had collected the full upstream
    // body before returning, the body would dump in one burst with near-zero gap.
    let body_start = Instant::now();
    let mut body = res.into_body().into_data_stream();
    let mut assembled = String::new();
    let mut saw_chunk1_at: Option<Duration> = None;
    let mut saw_chunk3_at: Option<Duration> = None;

    while let Some(frame) = body.next().await {
        let bytes = frame.expect("body frame");
        let s = String::from_utf8_lossy(&bytes);
        assembled.push_str(&s);
        if saw_chunk1_at.is_none() && assembled.contains("chunk-1") {
            saw_chunk1_at = Some(body_start.elapsed());
        }
        if saw_chunk3_at.is_none() && assembled.contains("chunk-3") {
            saw_chunk3_at = Some(body_start.elapsed());
        }
    }

    // All three chunks in order.
    let i1 = assembled
        .find("chunk-1")
        .expect("chunk-1 must be present");
    let i2 = assembled
        .find("chunk-2")
        .expect("chunk-2 must be present");
    let i3 = assembled
        .find("chunk-3")
        .expect("chunk-3 must be present");
    assert!(i1 < i2 && i2 < i3, "chunks must arrive in order: {assembled}");
    assert!(assembled.contains("[DONE]"));

    let t1 = saw_chunk1_at.expect("chunk-1 timing");
    let t3 = saw_chunk3_at.expect("chunk-3 timing");
    assert!(
        t1 < t3,
        "chunk-1 ({t1:?}) should be observed before chunk-3 ({t3:?})"
    );
    // Gap between first and last chunk should reflect upstream inter-chunk sleeps.
    // (argon2 auth cost is paid before headers; it does not inflate this gap.)
    let gap = t3.saturating_sub(t1);
    assert!(
        gap >= Duration::from_millis(50),
        "expected streaming gap between chunk-1 and chunk-3 ≥50ms, got {gap:?} \
         (near-zero gap would indicate full-body buffering before client read)"
    );
}

/// Wiremock path: assert gateway forwards configured upstream Authorization
/// (static multi-chunk SSE body; order preserved).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn chat_completions_forwards_upstream_authorization_wiremock() {
    let mock = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer from-env-config"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "data: {\"id\":\"a\"}\n\n\
                     data: {\"id\":\"b\"}\n\n\
                     data: {\"id\":\"c\"}\n\n\
                     data: [DONE]\n\n",
                ),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test()
        .await
        .with_upstream(mock.uri(), Some("Bearer from-env-config".into()));
    let (row, plaintext) = create_member_key(&state.db, "wiremock-user").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", format!("Bearer {plaintext}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"gpt-4o","messages":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 64)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(text.contains("\"id\":\"a\""));
    assert!(text.contains("\"id\":\"b\""));
    assert!(text.contains("\"id\":\"c\""));
    // Order
    assert!(text.find("\"id\":\"a\"").unwrap() < text.find("\"id\":\"b\"").unwrap());
    assert!(text.find("\"id\":\"b\"").unwrap() < text.find("\"id\":\"c\"").unwrap());

    // Drop app / complete mock expectations.
    tokio::time::sleep(Duration::from_millis(50)).await;
}

#[tokio::test]
async fn chat_completions_rejects_missing_bearer() {
    let state = AppState::new_for_test()
        .await
        .with_upstream("http://127.0.0.1:1", None);
    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn forward_byte_stream_helper_is_usable() {
    // Unit-ish: the public stream helper maps reqwest errors to io and builds a Body.
    use futures_util::stream;
    use tagw::proxy::stream::forward_byte_stream;

    let s = stream::iter(vec![
        Ok::<Bytes, reqwest::Error>(Bytes::from("one")),
        Ok(Bytes::from("two")),
    ]);
    let body = forward_byte_stream(s);
    let bytes = axum::body::to_bytes(body, 1024).await.unwrap();
    assert_eq!(&bytes[..], b"onetwo");
}
