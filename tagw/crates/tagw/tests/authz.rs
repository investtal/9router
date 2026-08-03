//! Dashboard auth + RBAC: unauthenticated 401, viewer 403 on mutate, admin 200.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tagw::admin::users::insert_user;
use tagw::app::build_app;
use tagw::auth::dashboard::Role;
use tagw::state::AppState;
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

fn set_cookie_from_response(res: &axum::response::Response) -> Option<String> {
    res.headers()
        .get_all(axum::http::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|s| {
            s.split(';')
                .next()
                .and_then(|pair| {
                    let pair = pair.trim();
                    if pair.starts_with("tagw_session=") {
                        Some(pair.to_string())
                    } else {
                        None
                    }
                })
        })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn unauthenticated_post_keys_returns_401() {
    let state = AppState::new_for_test().await;
    let app = build_app(state);

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/keys")
                .header("content-type", "application/json")
                .body(Body::from(json!({ "name": "k" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_post_keys_returns_403() {
    let state = AppState::new_for_test().await;
    insert_user(&state.db, "viewer1", "viewer-pass", Role::Viewer).expect("insert viewer");
    let app = build_app(state.clone());
    let cookie = state.test_session_cookie("viewer1");

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/keys")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(json!({ "name": "blocked" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn admin_post_keys_returns_200() {
    let state = AppState::new_for_test().await;
    let app = build_app(state.clone());
    let cookie = state.test_session_cookie("admin");

    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/keys")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(json!({ "name": "allowed" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body = read_json(res).await;
    assert_eq!(body["name"], "allowed");
    assert!(body["key"].as_str().unwrap().starts_with("sk-"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn login_sets_session_cookie_and_me_works() {
    let state = AppState::new_for_test().await;
    let app = build_app(state);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "username": "admin", "password": "admin" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = set_cookie_from_response(&res).expect("Set-Cookie tagw_session");
    let body = read_json(res).await;
    assert_eq!(body["username"], "admin");
    assert_eq!(body["role"], "admin");

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let me = read_json(res).await;
    assert_eq!(me["username"], "admin");
    assert_eq!(me["role"], "admin");

    // Logout clears cookie; subsequent me is 401.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/logout")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/me")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn viewer_can_list_providers_redacted() {
    let state = AppState::new_for_test().await;
    insert_user(&state.db, "viewer2", "pass", Role::Viewer).unwrap();
    let app = build_app(state.clone());
    let admin = state.test_session_cookie("admin");
    let viewer = state.test_session_cookie("viewer2");

    // Admin creates a provider.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/providers")
                .header("content-type", "application/json")
                .header("cookie", &admin)
                .body(Body::from(
                    json!({ "provider_type": "deepseek", "name": "DS" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Viewer lists via public path.
    let res = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/providers")
                .header("cookie", &viewer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let list = read_json(res).await;
    assert!(list.as_array().unwrap().iter().any(|p| p["name"] == "DS"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn bad_login_returns_401() {
    let state = AppState::new_for_test().await;
    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/auth/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    json!({ "username": "admin", "password": "wrong" }).to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn llm_v1_still_member_key_only_no_session() {
    let state = AppState::new_for_test().await;
    let app = build_app(state.clone());
    // Session cookie alone must not authorize /v1.
    let cookie = state.test_session_cookie("admin");
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/chat/completions")
                .header("cookie", cookie)
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"model":"gpt-4o","messages":[{"role":"user","content":"hi"}]}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}
