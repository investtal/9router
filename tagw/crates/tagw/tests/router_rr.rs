//! AccountRouter unit + integration: round-robin, skip disabled, fail-over.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::post;
use axum::Router;
use bytes::Bytes;
use futures_util::StreamExt;
use tagw::app::build_app;
use tagw::auth::member_key::create_member_key;
use tagw::cache::CachedAccount;
use tagw::router::{AccountRef, AccountRouter, MAX_FAILOVER_ATTEMPTS};
use tagw::state::{AppState, DEFAULT_POOL_KEY};
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn acct(id: &str, base: &str) -> AccountRef {
    AccountRef {
        account_id: id.into(),
        provider_id: "prov-1".into(),
        upstream_base: base.into(),
        auth_header: format!("Bearer {id}"),
        is_oauth: false,
    }
}

// ── Unit: pick order + skip disabled + should_failover ──────────────────────

#[test]
fn unit_pick_order_round_robin() {
    let router = AccountRouter::new();
    let pool = vec![
        acct("a", "http://a"),
        acct("b", "http://b"),
        acct("c", "http://c"),
    ];
    let ids: Vec<_> = (0..5)
        .map(|_| router.pick("default", &pool).unwrap().account_id)
        .collect();
    assert_eq!(ids, vec!["a", "b", "c", "a", "b"]);
}

#[test]
fn unit_should_failover_helper() {
    assert!(AccountRouter::should_failover(429));
    assert!(AccountRouter::should_failover(500));
    assert!(AccountRouter::should_failover(502));
    assert!(AccountRouter::should_failover(503));
    assert!(AccountRouter::should_failover(504));
    assert!(!AccountRouter::should_failover(200));
    assert!(!AccountRouter::should_failover(400));
    assert!(!AccountRouter::should_failover(401));
    assert!(!AccountRouter::should_failover(404));
    assert_eq!(MAX_FAILOVER_ATTEMPTS, 3);
}

#[test]
fn unit_disabled_accounts_skipped_via_cache() {
    let cache = tagw::cache::ConfigCache::new();
    cache.set_account_pool(
        DEFAULT_POOL_KEY,
        vec![
            CachedAccount {
                account: acct("enabled-1", "http://e1"),
                enabled: true,
            },
            CachedAccount {
                account: acct("disabled", "http://d"),
                enabled: false,
            },
            CachedAccount {
                account: acct("enabled-2", "http://e2"),
                enabled: true,
            },
        ],
    );

    let enabled = cache.enabled_accounts(DEFAULT_POOL_KEY);
    assert_eq!(enabled.len(), 2);
    assert_eq!(enabled[0].account_id, "enabled-1");
    assert_eq!(enabled[1].account_id, "enabled-2");

    let router = AccountRouter::new();
    for _ in 0..6 {
        let picked = router.pick(DEFAULT_POOL_KEY, &enabled).unwrap();
        assert_ne!(
            picked.account_id, "disabled",
            "disabled account must never be picked"
        );
    }
}

#[test]
fn unit_pick_none_on_empty_after_all_disabled() {
    let cache = tagw::cache::ConfigCache::new();
    cache.set_account_pool(
        DEFAULT_POOL_KEY,
        vec![CachedAccount {
            account: acct("only-disabled", "http://x"),
            enabled: false,
        }],
    );
    let enabled = cache.enabled_accounts(DEFAULT_POOL_KEY);
    assert!(enabled.is_empty());
    let router = AccountRouter::new();
    assert!(router.pick(DEFAULT_POOL_KEY, &enabled).is_none());
}

// ── Integration: two mock upstreams — first 429, second 200 stream ──────────

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failover_from_429_to_200_stream() {
    // Upstream A: always 429 (rate limited).
    let mock_a = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"error":"rate_limited"}"#),
        )
        .expect(1..)
        .mount(&mock_a)
        .await;

    // Upstream B: 200 SSE stream.
    let mock_b = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "data: {\"id\":\"ok-chunk\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n\
                     data: [DONE]\n\n",
                ),
        )
        .expect(1)
        .mount(&mock_b)
        .await;

    let state = AppState::new_for_test().await;
    // No TAGW_UPSTREAM — routing must come from the pool.
    state.cache.set_account_pool(
        DEFAULT_POOL_KEY,
        vec![
            CachedAccount {
                account: acct("acct-a", &mock_a.uri()),
                enabled: true,
            },
            CachedAccount {
                account: acct("acct-b", &mock_b.uri()),
                enabled: true,
            },
        ],
    );

    let (row, plaintext) = create_member_key(&state.db, "failover-user").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", format!("Bearer {plaintext}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        res.status(),
        StatusCode::OK,
        "client must get 200 after fail-over from 429"
    );
    let body = axum::body::to_bytes(res.into_body(), 1024 * 64)
        .await
        .unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("ok-chunk"),
        "client must receive stream from second account: {text}"
    );
    assert!(text.contains("[DONE]"));
    assert!(
        !text.contains("rate_limited"),
        "429 body must not be forwarded when fail-over succeeds"
    );
}

/// After a non-failover error (or exhausted attempts with only failing accounts),
/// the last upstream status is returned — but 200 success path is preferred.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pool_empty_falls_back_to_tagw_upstream() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"id":"fallback-ok"}"#),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test()
        .await
        .with_upstream(mock.uri(), Some("Bearer env-upstream".into()));
    // Pool deliberately empty.
    assert!(state.cache.enabled_accounts(DEFAULT_POOL_KEY).is_empty());

    let (row, plaintext) = create_member_key(&state.db, "fallback-user").unwrap();
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
    let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("fallback-ok"));
}

/// Disabled first account is skipped; only the second (enabled) is called.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_account_not_contacted() {
    let hits = Arc::new(AtomicUsize::new(0));
    let hits_disabled = Arc::clone(&hits);

    async fn disabled_handler(
        axum::extract::State(hits): axum::extract::State<Arc<AtomicUsize>>,
        _body: Bytes,
    ) -> impl IntoResponse {
        hits.fetch_add(1, Ordering::SeqCst);
        (StatusCode::OK, "should-not-be-hit")
    }

    let disabled_app = Router::new()
        .route("/v1/chat/completions", post(disabled_handler))
        .with_state(hits_disabled);
    let disabled_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let disabled_addr = disabled_listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(disabled_listener, disabled_app).await.ok();
    });

    let mock_ok = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(r#"{"id":"from-enabled"}"#),
        )
        .expect(1)
        .mount(&mock_ok)
        .await;

    let state = AppState::new_for_test().await;
    state.cache.set_account_pool(
        DEFAULT_POOL_KEY,
        vec![
            CachedAccount {
                account: acct("disabled-acct", &format!("http://{disabled_addr}")),
                enabled: false,
            },
            CachedAccount {
                account: acct("enabled-acct", &mock_ok.uri()),
                enabled: true,
            },
        ],
    );
    let (row, plaintext) = create_member_key(&state.db, "skip-disabled").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", format!("Bearer {plaintext}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"x","messages":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("from-enabled"));
    assert_eq!(
        hits.load(Ordering::SeqCst),
        0,
        "disabled upstream must never receive a request"
    );
}

/// After first response byte is committed we do not switch accounts mid-stream.
/// This is enforced by only deciding fail-over on status before `bytes_stream()`;
/// a 200 with body is always forwarded end-to-end.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_switch_after_first_byte_on_success_stream() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(
                    "data: {\"id\":\"only-one\"}\n\ndata: [DONE]\n\n",
                ),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let mock_bad = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("nope"))
        .expect(0)
        .mount(&mock_bad)
        .await;

    let state = AppState::new_for_test().await;
    state.cache.set_account_pool(
        DEFAULT_POOL_KEY,
        vec![
            CachedAccount {
                account: acct("good", &mock.uri()),
                enabled: true,
            },
            CachedAccount {
                account: acct("bad", &mock_bad.uri()),
                enabled: true,
            },
        ],
    );
    let (row, plaintext) = create_member_key(&state.db, "no-mid-switch").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", format!("Bearer {plaintext}"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"model":"x","stream":true,"messages":[]}"#))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(res.status(), StatusCode::OK);
    let mut body = res.into_body().into_data_stream();
    let mut assembled = String::new();
    while let Some(frame) = body.next().await {
        assembled.push_str(&String::from_utf8_lossy(&frame.unwrap()));
    }
    assert!(assembled.contains("only-one"));
    // mock_bad expect(0) verified on drop.
}
