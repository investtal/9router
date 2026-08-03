//! OAuth refresh: mock token endpoint → expired account → ensure_access_token refreshes once.
//! Also: proxy path ensures token pre-hop and force-refreshes on upstream 401.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration, Utc};
use serde_json::json;
use tagw::app::build_app;
use tagw::auth::member_key::create_member_key;
use tagw::oauth::codex::CodexProvider;
use tagw::oauth::types::{
    OAuthCredentials, OAuthProvider, TokenSet, ACCESS_TOKEN_REFRESH_SKEW_SECS,
};
use tagw::oauth::{
    ensure_access_token_with_client, insert_oauth_account, load_oauth_account_pools,
};
use tagw::state::{AppState, DEFAULT_POOL_KEY};
use tower::ServiceExt;
use wiremock::matchers::{body_string_contains, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Required test: expired token → ensure_access_token calls refresh once → new token in SQLite.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn expired_token_refreshes_once_and_stores_in_sqlite() {
    let mock = MockServer::start().await;

    // Codex refresh posts JSON { client_id, grant_type, refresh_token } to token URL.
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("refresh-old"))
        .and(body_string_contains("refresh_token"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({
                    "access_token": "access-new",
                    "refresh_token": "refresh-new",
                    "expires_in": 3600,
                    "token_type": "Bearer"
                })),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test().await;
    let http = state.http_client.clone();

    let expired_at = Utc::now() - Duration::seconds(30);
    let creds = OAuthCredentials {
        access_token: "access-old".into(),
        refresh_token: Some("refresh-old".into()),
        expires_at: Some(expired_at),
        base_url: Some("https://api.openai.com".into()),
        token_url: Some(format!("{}/oauth/token", mock.uri())),
        client_id: Some("test-client".into()),
        client_secret: None,
        extra: None,
    };

    let (_provider_id, account_id) =
        insert_oauth_account(&state.db, "codex", "codex-test", &creds).expect("insert oauth account");

    // Sanity: needs refresh with 120s skew.
    assert!(creds.needs_refresh(ACCESS_TOKEN_REFRESH_SKEW_SECS));

    let token = ensure_access_token_with_client(
        &state.db,
        &state.cache,
        &account_id,
        &http,
        false,
    )
    .await
    .expect("ensure_access_token");

    assert_eq!(token, "access-new");

    // New credentials persisted in SQLite.
    let stored_raw: String = state
        .db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT credentials_json FROM accounts WHERE id = ?1",
                rusqlite::params![account_id],
                |r| r.get::<_, String>(0),
            )
        })
        .expect("read credentials");
    let stored: OAuthCredentials = serde_json::from_str(&stored_raw).expect("parse credentials");
    assert_eq!(stored.access_token, "access-new");
    assert_eq!(stored.refresh_token.as_deref(), Some("refresh-new"));
    assert!(stored.expires_at.is_some());
    assert!(
        !stored.needs_refresh(ACCESS_TOKEN_REFRESH_SKEW_SECS),
        "fresh token should not need refresh"
    );

    // Cache pool carries the new Bearer token.
    let pools = state.cache.enabled_accounts(DEFAULT_POOL_KEY);
    let found = pools.iter().find(|a| a.account_id == account_id);
    assert!(found.is_some(), "oauth account in default pool");
    assert_eq!(found.unwrap().auth_header, "Bearer access-new");

    // Second ensure without force should NOT hit mock again (expect(1) enforces).
    let token2 = ensure_access_token_with_client(
        &state.db,
        &state.cache,
        &account_id,
        &http,
        false,
    )
    .await
    .expect("second ensure");
    assert_eq!(token2, "access-new");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn codex_exchange_code_with_mock() {
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({
                    "access_token": "exchanged-access",
                    "refresh_token": "exchanged-refresh",
                    "expires_in": 7200
                })),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let http = reqwest::Client::new();
    let provider = CodexProvider::new(http).with_endpoints(format!("{}/oauth/token", mock.uri()), None);
    let pkce = tagw::oauth::pkce::generate_pkce("http://127.0.0.1:20128/api/oauth/codex/callback");
    let tokens = provider
        .exchange_code("auth-code-xyz", &pkce)
        .await
        .expect("exchange");
    assert_eq!(tokens.access_token, "exchanged-access");
    assert_eq!(tokens.refresh_token.as_deref(), Some("exchanged-refresh"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oauth_start_returns_authorize_url_json() {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tagw::app::build_app;
    use tower::ServiceExt;

    let state = AppState::new_for_test()
        .await
        .with_public_base("http://127.0.0.1:20128");
    let app = build_app(state.clone());

    let res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/oauth/codex/start?redirect=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let url = v["authorize_url"].as_str().expect("authorize_url");
    assert!(url.contains("auth.openai.com") || url.contains("oauth/authorize"));
    assert!(url.contains("code_challenge"));
    assert!(url.contains("client_id"));
    assert_eq!(v["provider"], "codex");
    let state_param = v["state"].as_str().expect("state");
    assert!(!state_param.is_empty());
    // Pending map has the session.
    let pending = state.oauth_pending.lock().unwrap();
    assert!(pending.contains_key(state_param));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oauth_account_loads_into_pool() {
    let state = AppState::new_for_test().await;
    let creds = OAuthCredentials {
        access_token: "pool-token".into(),
        refresh_token: Some("rt".into()),
        expires_at: Some(Utc::now() + Duration::hours(1)),
        base_url: Some("https://api.openai.com".into()),
        token_url: None,
        client_id: None,
        client_secret: None,
        extra: None,
    };
    let (_pid, aid) = insert_oauth_account(&state.db, "codex", "pool-acct", &creds).unwrap();
    state.cache.reload(&state.db).unwrap();
    let enabled = state.cache.enabled_accounts(DEFAULT_POOL_KEY);
    let hit = enabled.iter().find(|a| a.account_id == aid).expect("in pool");
    assert_eq!(hit.auth_header, "Bearer pool-token");
    assert_eq!(hit.upstream_base, "https://api.openai.com");
    assert!(hit.is_oauth, "oauth pool entries must set is_oauth");

    let pools = load_oauth_account_pools(&state.db).unwrap();
    assert!(pools.contains_key(DEFAULT_POOL_KEY));
}

#[tokio::test]
async fn token_set_from_oauth_json() {
    let v = json!({
        "access_token": "a",
        "refresh_token": "r",
        "expires_in": 60
    });
    let t = TokenSet::from_oauth_json(&v).unwrap();
    assert_eq!(t.access_token, "a");
    assert_eq!(t.refresh_token.as_deref(), Some("r"));
    assert!(t.expires_at.is_some());
}

/// Proxy pre-hop: expired OAuth access token is refreshed before the upstream call
/// so the mock API only ever sees the new Bearer.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proxy_ensures_oauth_token_before_upstream() {
    let mock = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("refresh-expired"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({
                    "access_token": "access-refreshed",
                    "refresh_token": "refresh-expired",
                    "expires_in": 3600
                })),
        )
        .expect(1)
        .mount(&mock)
        .await;

    // Upstream must receive the refreshed token only (never the expired one).
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer access-refreshed"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"id":"ok","choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#),
        )
        .expect(1)
        .mount(&mock)
        .await;

    // Reject if stale Bearer is used (guards against missing ensure).
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer access-expired"))
        .respond_with(ResponseTemplate::new(401))
        .expect(0)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test().await;
    let creds = OAuthCredentials {
        access_token: "access-expired".into(),
        refresh_token: Some("refresh-expired".into()),
        expires_at: Some(Utc::now() - Duration::seconds(30)),
        base_url: Some(mock.uri()),
        token_url: Some(format!("{}/oauth/token", mock.uri())),
        client_id: Some("test-client".into()),
        client_secret: None,
        extra: None,
    };
    let (_pid, _aid) =
        insert_oauth_account(&state.db, "codex", "proxy-pre-ensure", &creds).unwrap();
    state.cache.reload(&state.db).unwrap();

    let (row, plaintext) = create_member_key(&state.db, "oauth-proxy-user").unwrap();
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
    let body = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
    assert!(String::from_utf8_lossy(&body).contains("\"id\":\"ok\""));
}

/// Proxy OAuth 401 path: force-refresh once, retry same account with new token,
/// never mid-body switch (retry is still pre-first-byte).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proxy_oauth_401_force_refresh_then_retry_same_account() {
    let mock = MockServer::start().await;

    // Access token is still "fresh" by expires_at so pre-hop ensure does not refresh.
    // Upstream rejects it once; after force-refresh, accepts the new token.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer access-stale"))
        .respond_with(
            ResponseTemplate::new(401)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"error":"invalid_token"}"#),
        )
        .expect(1)
        .mount(&mock)
        .await;

    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .and(body_string_contains("refresh-stale"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({
                    "access_token": "access-fresh",
                    "refresh_token": "refresh-stale",
                    "expires_in": 3600
                })),
        )
        .expect(1)
        .mount(&mock)
        .await;

    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer access-fresh"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(
                    r#"{"id":"after-retry","choices":[],"usage":{"prompt_tokens":2,"completion_tokens":1}}"#,
                ),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test().await;
    let creds = OAuthCredentials {
        access_token: "access-stale".into(),
        refresh_token: Some("refresh-stale".into()),
        expires_at: Some(Utc::now() + Duration::hours(1)),
        base_url: Some(mock.uri()),
        token_url: Some(format!("{}/oauth/token", mock.uri())),
        client_id: Some("test-client".into()),
        client_secret: None,
        extra: None,
    };
    let (_pid, aid) =
        insert_oauth_account(&state.db, "codex", "proxy-401-retry", &creds).unwrap();
    state.cache.reload(&state.db).unwrap();

    // Sanity: pool has oauth flag.
    let pool = state.cache.enabled_accounts(DEFAULT_POOL_KEY);
    let hit = pool.iter().find(|a| a.account_id == aid).expect("in pool");
    assert!(hit.is_oauth);
    assert_eq!(hit.auth_header, "Bearer access-stale");

    let (row, plaintext) = create_member_key(&state.db, "oauth-401-user").unwrap();
    state.cache.upsert(&row);

    let app = build_app(state.clone());
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
    let body = axum::body::to_bytes(res.into_body(), 64 * 1024).await.unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("after-retry"),
        "client must see success after same-account 401 retry"
    );

    // Force-refresh persisted the new token.
    let stored_raw: String = state
        .db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT credentials_json FROM accounts WHERE id = ?1",
                rusqlite::params![aid],
                |r| r.get::<_, String>(0),
            )
        })
        .unwrap();
    let stored: OAuthCredentials = serde_json::from_str(&stored_raw).unwrap();
    assert_eq!(stored.access_token, "access-fresh");
}
