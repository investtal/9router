//! API-key providers: admin CRUD → ConfigCache pools → proxy hits account base_url.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tagw::app::build_app;
use tagw::auth::member_key::create_member_key;
use tagw::providers::api_key::{
    create_account, create_provider, CreateAccountRequest, CreateProviderRequest,
};
use tagw::state::{AppState, ANTHROPIC_POOL_KEY, DEFAULT_POOL_KEY, OPENAI_COMPAT_POOL_KEY};
use tower::ServiceExt;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn admin_cookie(state: &AppState) -> String {
    state.test_session_cookie("admin")
}

async fn read_json(res: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap_or_else(|_| {
        panic!(
            "expected json body, got: {}",
            String::from_utf8_lossy(&body)
        )
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn create_provider_account_loads_pool_and_proxy_hits_base_url() {
    // Upstream wiremock: must receive the account API key, not the member key.
    let mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-upstream-provider-key"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"id":"from-api-key-provider","object":"chat.completion"}"#),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test().await;
    // No TAGW_UPSTREAM — routing must come from the loaded provider pool.
    assert!(state.cache.enabled_accounts(DEFAULT_POOL_KEY).is_empty());

    let app = build_app(state.clone());

    // 1) Create provider (openai_compat so base_url is explicit).
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .header("cookie", admin_cookie(&state))
                .uri("/api/admin/providers")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "provider_type": "openai_compat",
                        "name": "Wiremock Upstream"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "create provider");
    let provider = read_json(res).await;
    let provider_id = provider["id"].as_str().expect("provider id");
    assert_eq!(provider["kind"], "api_key");
    assert_eq!(provider["provider_type"], "openai_compat");
    assert_eq!(provider["enabled"], true);

    // 2) Create account with api_key + base_url = wiremock.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .header("cookie", admin_cookie(&state))
                .uri(format!("/api/admin/providers/{provider_id}/accounts"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "label": "primary",
                        "api_key": "sk-upstream-provider-key",
                        "base_url": mock.uri(),
                        "models": ["gpt-4o"]
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "create account");
    let account = read_json(res).await;
    assert_eq!(account["label"], "primary");
    assert_eq!(account["enabled"], true);
    assert_eq!(account["provider_id"], provider_id);
    // Secret redacted in response.
    let prefix = account["credentials"]["api_key_prefix"]
        .as_str()
        .unwrap_or("");
    assert!(
        prefix.starts_with("sk-upstr") || prefix.contains('…'),
        "api_key must be redacted, got {prefix}"
    );
    assert!(!prefix.contains("provider-key"));

    // 3) Cache must have the openai_compat pool (reload happens on mutate).
    let enabled = state.cache.enabled_accounts(OPENAI_COMPAT_POOL_KEY);
    assert_eq!(
        enabled.len(),
        1,
        "openai_compat pool should contain the new account"
    );
    assert_eq!(enabled[0].upstream_base, mock.uri());
    assert_eq!(
        enabled[0].auth_header,
        "Bearer sk-upstream-provider-key"
    );
    assert_eq!(enabled[0].provider_id, provider_id);

    // Provider-id pool also present.
    let by_provider = state.cache.enabled_accounts(provider_id);
    assert_eq!(by_provider.len(), 1);

    // 4) List providers returns nested account (redacted).
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .header("cookie", admin_cookie(&state))
                .uri("/api/admin/providers")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let list = read_json(res).await;
    let list = list.as_array().expect("providers array");
    assert!(!list.is_empty());
    let found = list
        .iter()
        .find(|p| p["id"] == provider_id)
        .expect("created provider in list");
    assert_eq!(found["accounts"].as_array().unwrap().len(), 1);

    // 5) Proxy with member key hits wiremock via account credentials.
    let (row, plaintext) = create_member_key(&state.db, "proxy-user").unwrap();
    state.cache.upsert(&row);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", format!("Bearer {plaintext}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(
        res.status(),
        StatusCode::OK,
        "proxy must route to provider account base_url"
    );
    let body = axum::body::to_bytes(res.into_body(), 1024).await.unwrap();
    let text = String::from_utf8_lossy(&body);
    assert!(
        text.contains("from-api-key-provider"),
        "expected upstream body, got: {text}"
    );
    // wiremock expect(1) verified on drop.
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disable_provider_removes_account_from_enabled_pool() {
    let mock = MockServer::start().await;
    // Should never be contacted once disabled.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(200).set_body_string(r#"{"id":"should-not"}"#))
        .expect(0)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test().await;
    let app = build_app(state.clone());

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .header("cookie", admin_cookie(&state))
                .uri("/api/admin/providers")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "provider_type": "deepseek",
                        "name": "DS"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let provider_id = read_json(res).await["id"].as_str().unwrap().to_string();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .header("cookie", admin_cookie(&state))
                .uri(format!("/api/admin/providers/{provider_id}/accounts"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "label": "a1",
                        "api_key": "sk-ds-key",
                        "base_url": mock.uri()
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(state.cache.enabled_accounts(DEFAULT_POOL_KEY).len(), 1);

    // Disable provider → pool empty for default.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .header("cookie", admin_cookie(&state))
                .uri(format!("/api/admin/providers/{provider_id}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "enabled": false }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        state.cache.enabled_accounts(DEFAULT_POOL_KEY).is_empty(),
        "disabled provider accounts must leave the enabled default pool"
    );

    // Re-enable via account path still blocked if provider disabled — re-enable provider.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .header("cookie", admin_cookie(&state))
                .uri(format!("/api/admin/providers/{provider_id}"))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "enabled": true }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(state.cache.enabled_accounts(DEFAULT_POOL_KEY).len(), 1);

    // Disable account only.
    let account_id = {
        let list = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .header("cookie", admin_cookie(&state))
                    .uri("/api/admin/providers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let list = read_json(list).await;
        list.as_array()
            .unwrap()
            .iter()
            .find(|p| p["id"] == provider_id)
            .unwrap()["accounts"][0]["id"]
            .as_str()
            .unwrap()
            .to_string()
    };

    let res = app
        .oneshot(
            Request::builder()
                .method("PATCH")
                .header("cookie", admin_cookie(&state))
                .uri(format!(
                    "/api/admin/providers/{provider_id}/accounts/{account_id}"
                ))
                .header("content-type", "application/json")
                .body(Body::from(json!({ "enabled": false }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(
        state.cache.enabled_accounts(DEFAULT_POOL_KEY).is_empty(),
        "disabled account must leave the enabled default pool"
    );
}

#[tokio::test]
async fn create_provider_rejects_unknown_type() {
    let state = AppState::new_for_test().await;
    let app = build_app(state.clone());
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .header("cookie", admin_cookie(&state))
                .uri("/api/admin/providers")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "provider_type": "nope", "name": "x" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn openai_compat_account_requires_base_url() {
    let state = AppState::new_for_test().await;
    let app = build_app(state.clone());

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .header("cookie", admin_cookie(&state))
                .uri("/api/admin/providers")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "provider_type": "openai_compat",
                        "name": "No Base"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    let provider_id = read_json(res).await["id"].as_str().unwrap().to_string();

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .header("cookie", admin_cookie(&state))
                .uri(format!("/api/admin/providers/{provider_id}/accounts"))
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({
                        "label": "a",
                        "api_key": "sk-x"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

/// OpenAI chat must not RR into anthropic accounts; Messages must not hit glm.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn openai_and_anthropic_pools_do_not_cross_contaminate() {
    let glm_mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(header("authorization", "Bearer sk-glm-only"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(r#"{"id":"from-glm","object":"chat.completion"}"#),
        )
        .expect(1)
        .mount(&glm_mock)
        .await;

    let ant_mock = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "sk-ant-only"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(
                    r#"{"id":"msg_from_ant","type":"message","role":"assistant","model":"claude-3","content":[],"usage":{"input_tokens":1,"output_tokens":1}}"#,
                ),
        )
        .expect(1)
        .mount(&ant_mock)
        .await;

    // Cross-path mocks: must never be hit.
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .respond_with(ResponseTemplate::new(500).set_body_string("wrong path on anthropic mock"))
        .expect(0)
        .mount(&ant_mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(500).set_body_string("wrong path on glm mock"))
        .expect(0)
        .mount(&glm_mock)
        .await;

    let state = AppState::new_for_test()
        .await
        // Poisoned fallback — pools must route without TAGW_UPSTREAM.
        .with_upstream("http://127.0.0.1:1", Some("Bearer wrong".into()));

    let glm_prov = create_provider(
        &state.db,
        &CreateProviderRequest {
            provider_type: "glm".into(),
            name: "GLM".into(),
            enabled: Some(true),
            config_json: None,
        },
    )
    .unwrap();
    create_account(
        &state.db,
        &glm_prov.id,
        &CreateAccountRequest {
            label: "glm-main".into(),
            api_key: "sk-glm-only".into(),
            base_url: Some(glm_mock.uri()),
            models: None,
            enabled: Some(true),
        },
    )
    .unwrap();

    let ant_prov = create_provider(
        &state.db,
        &CreateProviderRequest {
            provider_type: "anthropic".into(),
            name: "Anthropic".into(),
            enabled: Some(true),
            config_json: None,
        },
    )
    .unwrap();
    create_account(
        &state.db,
        &ant_prov.id,
        &CreateAccountRequest {
            label: "ant-main".into(),
            api_key: "sk-ant-only".into(),
            base_url: Some(ant_mock.uri()),
            models: None,
            enabled: Some(true),
        },
    )
    .unwrap();
    state.cache.reload(&state.db).unwrap();

    let openai_pool = state.cache.enabled_accounts(OPENAI_COMPAT_POOL_KEY);
    let anthropic_pool = state.cache.enabled_accounts(ANTHROPIC_POOL_KEY);
    assert_eq!(openai_pool.len(), 1, "only glm in openai_compat");
    assert_eq!(openai_pool[0].upstream_base, glm_mock.uri());
    assert_eq!(anthropic_pool.len(), 1, "only anthropic in anthropic pool");
    assert_eq!(anthropic_pool[0].upstream_base, ant_mock.uri());
    assert!(
        openai_pool
            .iter()
            .all(|a| a.account_id != anthropic_pool[0].account_id),
        "pools must be disjoint"
    );

    let (row, plaintext) = create_member_key(&state.db, "pool-iso").unwrap();
    state.cache.upsert(&row);
    let app = build_app(state);

    // OpenAI chat → glm mock only.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("authorization", format!("Bearer {plaintext}"))
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = axum::body::to_bytes(res.into_body(), 1024 * 64)
        .await
        .unwrap();
    assert!(
        String::from_utf8_lossy(&body).contains("from-glm"),
        "OpenAI path must hit glm, got {}",
        String::from_utf8_lossy(&body)
    );

    // Anthropic Messages → anthropic mock only.
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
    assert!(
        String::from_utf8_lossy(&body).contains("msg_from_ant"),
        "Anthropic path must hit anthropic, got {}",
        String::from_utf8_lossy(&body)
    );
}
