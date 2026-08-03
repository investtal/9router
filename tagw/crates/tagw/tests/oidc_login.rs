//! OIDC (Keycloak) login: mock token endpoint → session cookie; role map.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use serde_json::{json, Value};
use tagw::app::build_app;
use tagw::auth::oidc::{
    load_user_by_oidc_sub, save_oidc_config, OidcConfig, TAGW_ADMIN_ROLE,
};
use tagw::state::AppState;
use tower::ServiceExt;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn b64url_json(v: &Value) -> String {
    URL_SAFE_NO_PAD.encode(v.to_string().as_bytes())
}

/// Unsigned JWT for tests (payload only is consumed by decode_jwt_payload).
fn make_id_token(claims: Value) -> String {
    let header = b64url_json(&json!({"alg": "none", "typ": "JWT"}));
    let payload = b64url_json(&claims);
    format!("{header}.{payload}.sig")
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

fn set_cookie_session(res: &axum::response::Response) -> Option<String> {
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

async fn enable_oidc(state: &AppState, mock_uri: &str, redirect_uri: &str) {
    let issuer = format!("{mock_uri}/realms/tagw");
    save_oidc_config(
        &state.db,
        &OidcConfig {
            enabled: true,
            issuer,
            client_id: "tagw-dashboard".into(),
            client_secret: "secret".into(),
            redirect_uri: redirect_uri.into(),
        },
    )
    .expect("save oidc config");
}

fn mount_token(
    mock: &MockServer,
    claims: Value,
) -> impl std::future::Future<Output = ()> + '_ {
    let id_token = make_id_token(claims);
    async move {
        Mock::given(method("POST"))
            .and(path("/realms/tagw/protocol/openid-connect/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("content-type", "application/json")
                    .set_body_json(json!({
                        "access_token": "access-mock",
                        "id_token": id_token,
                        "token_type": "Bearer",
                        "expires_in": 300
                    })),
            )
            .expect(1)
            .mount(mock)
            .await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oidc_code_exchange_creates_viewer_session() {
    let mock = MockServer::start().await;
    let claims = json!({
        "sub": "kc-sub-viewer-1",
        "preferred_username": "alice",
        "email": "alice@example.com",
        "realm_access": { "roles": ["offline_access", "uma_authorization"] }
    });
    mount_token(&mock, claims).await;

    let state = AppState::new_for_test().await;
    let redirect_uri = "http://127.0.0.1:20128/api/auth/oidc/callback";
    enable_oidc(&state, &mock.uri(), redirect_uri).await;
    let app = build_app(state.clone());

    // Start (no redirect) → state + authorize_url.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/oidc/start?redirect=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let start = read_json(res).await;
    let state_param = start["state"].as_str().expect("state");
    let authorize_url = start["authorize_url"].as_str().expect("authorize_url");
    assert!(
        authorize_url.contains("/protocol/openid-connect/auth"),
        "authorize url: {authorize_url}"
    );
    assert!(authorize_url.contains("client_id=tagw-dashboard"));

    // Callback with code → session cookie + user JSON.
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/auth/oidc/callback?code=auth-code-1&state={state_param}&json=true"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = set_cookie_session(&res).expect("Set-Cookie tagw_session");
    let body = read_json(res).await;
    assert_eq!(body["username"], "alice");
    assert_eq!(body["role"], "viewer");

    // Linked in DB by oidc_sub.
    let linked = load_user_by_oidc_sub(&state.db, "kc-sub-viewer-1")
        .unwrap()
        .expect("user by sub");
    assert_eq!(linked.username, "alice");
    assert_eq!(linked.role, tagw::auth::dashboard::Role::Viewer);

    // Session works on /api/auth/me.
    let res = app
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
    assert_eq!(me["username"], "alice");
    assert_eq!(me["role"], "viewer");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oidc_tagw_admin_role_maps_to_admin() {
    let mock = MockServer::start().await;
    let claims = json!({
        "sub": "kc-sub-admin-1",
        "preferred_username": "bob-admin",
        "realm_access": { "roles": [TAGW_ADMIN_ROLE, "default-roles-tagw"] }
    });
    mount_token(&mock, claims).await;

    let state = AppState::new_for_test().await;
    let redirect_uri = "http://127.0.0.1:20128/api/auth/oidc/callback";
    enable_oidc(&state, &mock.uri(), redirect_uri).await;
    let app = build_app(state.clone());

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/oidc/start?redirect=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let start = read_json(res).await;
    let state_param = start["state"].as_str().unwrap();

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/auth/oidc/callback?code=auth-code-admin&state={state_param}&json=true"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let cookie = set_cookie_session(&res).expect("session cookie");
    let body = read_json(res).await;
    assert_eq!(body["role"], "admin");
    assert_eq!(body["username"], "bob-admin");

    // Admin can POST keys.
    let res = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/keys")
                .header("content-type", "application/json")
                .header("cookie", cookie)
                .body(Body::from(json!({ "name": "from-oidc" }).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oidc_disabled_start_fails_basic_login_still_works() {
    let state = AppState::new_for_test().await;
    // Default: OIDC disabled (no settings).
    let app = build_app(state);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/auth/oidc/start?redirect=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Basic auth still works.
    let res = app
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
    assert!(set_cookie_session(&res).is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn oidc_relink_same_sub_updates_role() {
    use wiremock::matchers::body_string_contains;

    let mock = MockServer::start().await;
    let claims_viewer = json!({
        "sub": "kc-sub-promote",
        "preferred_username": "promo",
        "realm_access": { "roles": [] }
    });
    let claims_admin = json!({
        "sub": "kc-sub-promote",
        "preferred_username": "promo",
        "realm_access": { "roles": [TAGW_ADMIN_ROLE] }
    });

    // Distinguish responses by authorization code in the form body.
    Mock::given(method("POST"))
        .and(path("/realms/tagw/protocol/openid-connect/token"))
        .and(body_string_contains("code=c1"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({
                    "access_token": "a1",
                    "id_token": make_id_token(claims_viewer),
                    "token_type": "Bearer",
                    "expires_in": 300
                })),
        )
        .expect(1)
        .mount(&mock)
        .await;
    Mock::given(method("POST"))
        .and(path("/realms/tagw/protocol/openid-connect/token"))
        .and(body_string_contains("code=c2"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_json(json!({
                    "access_token": "a2",
                    "id_token": make_id_token(claims_admin),
                    "token_type": "Bearer",
                    "expires_in": 300
                })),
        )
        .expect(1)
        .mount(&mock)
        .await;

    let state = AppState::new_for_test().await;
    enable_oidc(
        &state,
        &mock.uri(),
        "http://127.0.0.1:20128/api/auth/oidc/callback",
    )
    .await;
    let app = build_app(state.clone());

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/oidc/start?redirect=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let st1 = read_json(res).await["state"].as_str().unwrap().to_string();
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/auth/oidc/callback?code=c1&state={st1}&json=true"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(read_json(res).await["role"], "viewer");

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/auth/oidc/start?redirect=false")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let st2 = read_json(res).await["state"].as_str().unwrap().to_string();
    let res = app
        .oneshot(
            Request::builder()
                .uri(format!(
                    "/api/auth/oidc/callback?code=c2&state={st2}&json=true"
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = read_json(res).await;
    assert_eq!(body["role"], "admin");
    assert_eq!(body["username"], "promo");

    let linked = load_user_by_oidc_sub(&state.db, "kc-sub-promote")
        .unwrap()
        .unwrap();
    assert_eq!(linked.role, tagw::auth::dashboard::Role::Admin);
}
