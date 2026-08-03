//! Quota tracker: provider quota_json + derived 30d usage.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration, Utc};
use serde_json::{json, Value};
use tagw::app::build_app;
use tagw::providers::api_key::{create_account, create_provider, CreateAccountRequest, CreateProviderRequest};
use tagw::state::AppState;
use tagw::usage::query::{insert_request_log, RequestLogRow};
use tower::ServiceExt;

async fn read_json(res: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quota_merges_provider_snapshot_and_derived_usage() {
    let state = AppState::new_for_test().await;

    let prov = create_provider(
        &state.db,
        &CreateProviderRequest {
            provider_type: "openai_compat".into(),
            name: "Up".into(),
            enabled: Some(true),
            config_json: None,
        },
    )
    .expect("provider");
    let acct = create_account(
        &state.db,
        &prov.id,
        &CreateAccountRequest {
            label: "main".into(),
            api_key: "sk-test-quota".into(),
            base_url: Some("https://example.com".into()),
            models: None,
            enabled: Some(true),
        },
    )
    .expect("account");

    // Provider-populated quota snapshot
    state
        .db
        .with_conn(|c| {
            c.execute(
                "UPDATE accounts SET quota_json = ?1 WHERE id = ?2",
                rusqlite::params![
                    json!({"remaining_requests": 42, "reset_at": "2026-09-01T00:00:00Z"}).to_string(),
                    acct.id
                ],
            )
        })
        .unwrap();

    let now = Utc::now();
    insert_request_log(
        &state.db,
        &RequestLogRow {
            id: "q1".into(),
            created_at: (now - Duration::days(1)).to_rfc3339(),
            member_key_id: Some("k1".into()),
            provider_id: Some(prov.id.clone()),
            account_id: Some(acct.id.clone()),
            model: Some("gpt-4o".into()),
            tool: Some("openai".into()),
            status: Some(200),
            prompt_tokens: 11,
            completion_tokens: 22,
            cached_tokens: 1,
            cost_est: 0.05,
            latency_ms: Some(50),
            ttft_ms: Some(10),
            usage_incomplete: false,
            error: None,
        },
    )
    .unwrap();
    // Outside 30d — ignored for derived
    insert_request_log(
        &state.db,
        &RequestLogRow {
            id: "q-old".into(),
            created_at: (now - Duration::days(40)).to_rfc3339(),
            member_key_id: None,
            provider_id: Some(prov.id.clone()),
            account_id: Some(acct.id.clone()),
            model: Some("gpt-4o".into()),
            tool: Some("openai".into()),
            status: Some(200),
            prompt_tokens: 999,
            completion_tokens: 999,
            cached_tokens: 0,
            cost_est: 9.0,
            latency_ms: None,
            ttft_ms: None,
            usage_incomplete: false,
            error: None,
        },
    )
    .unwrap();

    let acct2 = create_account(
        &state.db,
        &prov.id,
        &CreateAccountRequest {
            label: "spare".into(),
            api_key: "sk-spare".into(),
            base_url: Some("https://example.com".into()),
            models: None,
            enabled: Some(true),
        },
    )
    .expect("account2");

    let app = build_app(state.clone());
    let cookie = state.test_session_cookie("admin");

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/quota")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = read_json(res).await;
    let accounts = body["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 2);

    let main = accounts
        .iter()
        .find(|a| a["account_id"] == acct.id)
        .expect("main account");
    assert_eq!(main["source"], "provider");
    assert_eq!(main["quota_json"]["remaining_requests"], 42);
    assert_eq!(main["derived"]["request_count"], 1);
    assert_eq!(main["derived"]["prompt_tokens"], 11);
    assert_eq!(main["derived"]["completion_tokens"], 22);
    assert!((main["derived"]["cost_est"].as_f64().unwrap() - 0.05).abs() < 1e-9);
    assert_eq!(main["derived"]["window_days"], 30);

    let spare = accounts
        .iter()
        .find(|a| a["account_id"] == acct2.id)
        .expect("spare");
    assert_eq!(spare["source"], "derived");
    assert_eq!(spare["derived"]["request_count"], 0);

    // Auth required
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/quota")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
