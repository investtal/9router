//! Integration: Anthropic Messages streaming proxy (Claude Code path).
//!
//! Wiremock Anthropic SSE fixture → gateway streams through with member key
//! (Bearer preferred; x-api-key also accepted).

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
use tagw::cache::CachedAccount;
use tagw::providers::api_key::{
    create_account, create_provider, CreateAccountRequest, CreateProviderRequest,
};
use tagw::router::AccountRef;
use tagw::state::{AppState, ANTHROPIC_POOL_KEY, DEFAULT_POOL_KEY};
use tower::ServiceExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Minimal Anthropic Messages SSE body (message_start → deltas → message_delta usage → stop).
const ANTHROPIC_SSE: &str = "\
event: message_start\n\
data: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-sonnet-4-20250514\",\"content\":[],\"usage\":{\"input_tokens\":12,\"output_tokens\":0}}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\
\n\
event: content_block_delta\n\
data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" world\"}}\n\
\n\
event: message_delta\n\
data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":4}}\n\
\n\
event: message_stop\n\
data: {\"type\":\"message_stop\"}\n\
\n";

/// Spawn a real streaming mock with inter-chunk delays (wiremock cannot delay mid-body).
async fn spawn_delayed_anthropic_sse_upstream() -> String {
    async fn handler(headers: HeaderMap, _body: Bytes) -> impl IntoResponse {
        // Anthropic API-key accounts are forwarded as x-api-key, not member key.
        let x_api = headers
            .get("x-api-key")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert_eq!(
            x_api, "sk-ant-upstream-secret",
            "upstream must receive account x-api-key, got {x_api:?}"
        );
        // Client member auth must not leak.
        let auth = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok());
        assert!(
            auth.is_none() || !auth.unwrap().contains("sk-"),
            "member bearer must not be forwarded as Authorization: {auth:?}"
        );

        let stream = futures_util::stream::unfold(0u8, |i| async move {
            if i >= 3 {
                return None;
            }
            if i > 0 {
                tokio::time::sleep(Duration::from_millis(40)).await;
            }
            let chunk = match i {
                0 => Bytes::from(
                    "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_a\",\"model\":\"claude-3\",\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}}\n\n",
                ),
                1 => Bytes::from(
                    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n\n",
                ),
                _ => Bytes::from(
                    "event: message_delta\ndata: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
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

    let app = Router::new().route("/v1/messages", post(handler));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.ok();
    });
    format!("http://{addr}")
}

/// Wiremock Anthropic SSE fixture → gateway streams through with member Bearer key.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_streams_anthropic_sse_with_member_bearer() {
    let mock = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-from-env"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(ANTHROPIC_SSE),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test()
        .await
        // Dev fallback: raw key becomes x-api-key on the Anthropic path.
        .with_upstream(mock.uri(), Some("sk-ant-from-env".into()));
    let (row, plaintext) = create_member_key(&state.db, "claude-code-user").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {plaintext}"))
                .header("content-type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .body(Body::from(
                    r#"{"model":"claude-sonnet-4-20250514","max_tokens":64,"messages":[{"role":"user","content":"hi"}],"stream":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 64)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("message_start"),
        "expected Anthropic SSE event, got: {text}"
    );
    assert!(text.contains("content_block_delta"));
    assert!(text.contains("message_stop"));
    assert!(text.contains("Hello"));
    assert!(
        text.find("message_start").unwrap() < text.find("message_stop").unwrap(),
        "events must preserve order"
    );

    tokio::time::sleep(Duration::from_millis(50)).await;
}

/// Member key via Anthropic `x-api-key` header (Claude Code default style).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_accepts_member_key_as_x_api_key() {
    let mock = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(ANTHROPIC_SSE),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test()
        .await
        .with_upstream(mock.uri(), Some("sk-ant-upstream".into()));
    let (row, plaintext) = create_member_key(&state.db, "x-api-key-user").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("x-api-key", &plaintext)
                .header("content-type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .body(Body::from(
                    r#"{"model":"claude-3","max_tokens":16,"messages":[],"stream":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 64)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("message_start"));
}

#[tokio::test]
async fn messages_rejects_missing_auth() {
    let state = AppState::new_for_test()
        .await
        .with_upstream("http://127.0.0.1:1", None);
    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("content-type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .body(Body::from("{}"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

/// Prefer anthropic pool accounts over default / TAGW_UPSTREAM.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_prefers_anthropic_pool_account() {
    let mock = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-pool-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(ANTHROPIC_SSE),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test()
        .await
        // Poisoned fallback — must not be used when anthropic pool has accounts.
        .with_upstream("http://127.0.0.1:1", Some("Bearer wrong".into()));

    state.cache.set_account_pool(
        ANTHROPIC_POOL_KEY,
        vec![CachedAccount {
            account: AccountRef {
                account_id: "acct-ant".into(),
                provider_id: "prov-ant".into(),
                upstream_base: mock.uri(),
                auth_header: "Bearer sk-ant-pool-key".into(),
                is_oauth: false,
            },
            enabled: true,
        }],
    );
    // Default pool empty — anthropic pool must be enough.
    assert!(state.cache.enabled_accounts(DEFAULT_POOL_KEY).is_empty());
    assert_eq!(state.cache.enabled_accounts(ANTHROPIC_POOL_KEY).len(), 1);

    let (row, plaintext) = create_member_key(&state.db, "pool-user").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {plaintext}"))
                .header("content-type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .body(Body::from(
                    r#"{"model":"claude-3","max_tokens":8,"messages":[{"role":"user","content":"x"}],"stream":true}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 64)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("message_start"));
}

/// Creating an anthropic api_key provider populates ANTHROPIC_POOL_KEY.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn anthropic_provider_loads_into_anthropic_pool() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-real"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(
                    r#"{"id":"msg_x","type":"message","role":"assistant","model":"claude-3","content":[],"usage":{"input_tokens":5,"output_tokens":1}}"#,
                ),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test().await;
    let provider = create_provider(
        &state.db,
        &CreateProviderRequest {
            provider_type: "anthropic".into(),
            name: "Anthropic Official".into(),
            enabled: Some(true),
            config_json: None,
        },
    )
    .unwrap();
    create_account(
        &state.db,
        &provider.id,
        &CreateAccountRequest {
            label: "main".into(),
            api_key: "sk-ant-real".into(),
            base_url: Some(mock.uri()),
            models: None,
            enabled: Some(true),
        },
    )
    .unwrap();
    state.cache.reload(&state.db).unwrap();

    let ant = state.cache.enabled_accounts(ANTHROPIC_POOL_KEY);
    assert_eq!(ant.len(), 1, "anthropic type must join anthropic pool");
    assert_eq!(ant[0].upstream_base, mock.uri());
    assert!(
        state
            .cache
            .enabled_accounts(DEFAULT_POOL_KEY)
            .iter()
            .any(|a| a.account_id == ant[0].account_id),
        "also in default pool"
    );

    let (row, plaintext) = create_member_key(&state.db, "ant-crud-user").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {plaintext}"))
                .header("content-type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .body(Body::from(
                    r#"{"model":"claude-3","max_tokens":8,"messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 64)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("msg_x"));
}

/// count_tokens passthrough when wired.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn count_tokens_passthrough() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages/count_tokens"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"input_tokens":42}"#),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test()
        .await
        .with_upstream(mock.uri(), Some("sk-ant-count".into()));
    let (row, plaintext) = create_member_key(&state.db, "count-user").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages/count_tokens")
                .header("authorization", format!("Bearer {plaintext}"))
                .header("content-type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .body(Body::from(
                    r#"{"model":"claude-3","messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024)
        .await
        .unwrap();
    assert!(String::from_utf8_lossy(&body).contains("42"));
}

/// Delayed SSE proves we do not full-body-buffer before client read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn messages_streams_chunks_without_buffering_all() {
    let upstream = spawn_delayed_anthropic_sse_upstream().await;

    let state = AppState::new_for_test().await;
    state.cache.set_account_pool(
        ANTHROPIC_POOL_KEY,
        vec![CachedAccount {
            account: AccountRef {
                account_id: "delay-acct".into(),
                provider_id: "prov".into(),
                upstream_base: upstream,
                auth_header: "Bearer sk-ant-upstream-secret".into(),
                is_oauth: false,
            },
            enabled: true,
        }],
    );
    let (row, plaintext) = create_member_key(&state.db, "stream-ant").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/messages")
                .header("authorization", format!("Bearer {plaintext}"))
                .header("content-type", "application/json")
                .header("anthropic-version", "2023-06-01")
                .body(Body::from(
                    r#"{"model":"claude-3","max_tokens":32,"stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);

    let body_start = Instant::now();
    let mut body = res.into_body().into_data_stream();
    let mut assembled = String::new();
    let mut saw_start_at: Option<Duration> = None;
    let mut saw_stop_at: Option<Duration> = None;

    while let Some(frame) = body.next().await {
        let bytes = frame.expect("body frame");
        let s = String::from_utf8_lossy(&bytes);
        assembled.push_str(&s);
        if saw_start_at.is_none() && assembled.contains("message_start") {
            saw_start_at = Some(body_start.elapsed());
        }
        if saw_stop_at.is_none() && assembled.contains("message_stop") {
            saw_stop_at = Some(body_start.elapsed());
        }
    }

    assert!(assembled.contains("message_start"));
    assert!(assembled.contains("message_stop"));
    let t0 = saw_start_at.expect("message_start timing");
    let t1 = saw_stop_at.expect("message_stop timing");
    let gap = t1.saturating_sub(t0);
    assert!(
        gap >= Duration::from_millis(50),
        "expected streaming gap ≥50ms, got {gap:?} (near-zero would mean full-body buffer)"
    );
}
