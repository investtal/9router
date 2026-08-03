//! Usage overview, request filters, and member breakdown APIs.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use chrono::{Duration, Utc};
use serde_json::Value;
use tagw::app::build_app;
use tagw::auth::member_key::create_member_key;
use tagw::state::AppState;
use tagw::usage::query::{insert_request_log, RequestLogRow};
use tower::ServiceExt;

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

fn seed_log(
    id: &str,
    created_at: &str,
    member_key_id: &str,
    model: &str,
    status: i32,
    prompt: i64,
    completion: i64,
    cost: f64,
) -> RequestLogRow {
    RequestLogRow {
        id: id.into(),
        created_at: created_at.into(),
        member_key_id: Some(member_key_id.into()),
        provider_id: Some("prov-1".into()),
        account_id: Some("acct-1".into()),
        model: Some(model.into()),
        tool: Some("openai".into()),
        status: Some(status),
        prompt_tokens: prompt,
        completion_tokens: completion,
        cached_tokens: 0,
        cost_est: cost,
        latency_ms: Some(100),
        ttft_ms: Some(20),
        usage_incomplete: false,
        error: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn usage_overview_and_member_filters() {
    let state = AppState::new_for_test().await;
    let (key_a, _) = create_member_key(&state.db, "alice").expect("key a");
    let (key_b, _) = create_member_key(&state.db, "bob").expect("key b");

    let now = Utc::now();
    let recent = (now - Duration::hours(1)).to_rfc3339();
    let mid = (now - Duration::days(2)).to_rfc3339();
    let old = (now - Duration::days(10)).to_rfc3339();

    insert_request_log(
        &state.db,
        &seed_log("r1", &recent, &key_a.id, "gpt-4o", 200, 10, 20, 0.01),
    )
    .unwrap();
    insert_request_log(
        &state.db,
        &seed_log("r2", &mid, &key_a.id, "gpt-4o-mini", 200, 5, 5, 0.002),
    )
    .unwrap();
    insert_request_log(
        &state.db,
        &seed_log("r3", &mid, &key_b.id, "gpt-4o", 500, 1, 0, 0.0),
    )
    .unwrap();
    // Outside 7d window — must not appear in overview/members for range=7d
    insert_request_log(
        &state.db,
        &seed_log("r4", &old, &key_a.id, "gpt-4o", 200, 100, 100, 1.0),
    )
    .unwrap();

    let app = build_app(state.clone());
    let cookie = state.test_session_cookie("admin");

    // Overview 7d: r1+r2+r3 (not r4)
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/usage/overview?range=7d")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = read_json(res).await;
    assert_eq!(body["request_count"], 3);
    assert_eq!(body["prompt_tokens"], 16);
    assert_eq!(body["completion_tokens"], 25);
    assert!((body["cost_est"].as_f64().unwrap() - 0.012).abs() < 1e-9);

    // Invalid range
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/usage/overview?range=1y")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Requests filtered by member + model
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/usage/requests?member_key_id={}&model=gpt-4o&limit=10",
                    key_a.id
                ))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = read_json(res).await;
    let items = body["items"].as_array().unwrap();
    // r1 (recent) + r4 (old) — both gpt-4o for key_a within default 30d
    assert_eq!(items.len(), 2);
    assert!(items.iter().all(|i| i["model"] == "gpt-4o"));
    assert!(items.iter().all(|i| i["member_key_id"] == key_a.id));

    // Status filter
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/usage/requests?status=500")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = read_json(res).await;
    assert_eq!(body["items"].as_array().unwrap().len(), 1);
    assert_eq!(body["items"][0]["id"], "r3");

    // Members cells
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/usage/members?range=7d")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = read_json(res).await;
    let cells = body.as_array().unwrap();
    assert!(cells.len() >= 2);
    let alice_4o = cells.iter().find(|c| {
        c["member_key_id"] == key_a.id && c["model"] == "gpt-4o" && c["member_name"] == "alice"
    });
    assert!(alice_4o.is_some(), "alice×gpt-4o cell missing: {cells:?}");
    assert_eq!(alice_4o.unwrap()["request_count"], 1); // only r1 in 7d

    // Member detail
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/usage/members/{}?range=7d", key_a.id))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = read_json(res).await;
    assert_eq!(body["member_key_id"], key_a.id);
    assert_eq!(body["member_name"], "alice");
    assert_eq!(body["request_count"], 2); // r1 + r2
    assert!(body["by_model"].as_array().unwrap().len() >= 2);
    assert!(!body["recent"].as_array().unwrap().is_empty());

    // Unauthenticated
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/usage/overview?range=today")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
