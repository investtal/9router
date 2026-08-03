//! SPA static file serving: ServeDir + index.html fallback; API takes precedence.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use tagw::app::{build_app, build_app_with_static};
use tagw::state::AppState;

async fn body_text(res: axum::response::Response) -> String {
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn spa_fallback_serves_index_for_client_routes() {
    let dir = tempfile::tempdir().unwrap();
    let index = dir.path().join("index.html");
    let asset = dir.path().join("assets");
    std::fs::create_dir_all(&asset).unwrap();
    std::fs::write(&index, b"<html><body>tagw-spa</body></html>").unwrap();
    std::fs::write(asset.join("app.js"), b"console.log('tagw')").unwrap();

    let state = AppState::new_for_test().await;
    let app = build_app_with_static(state, dir.path().to_path_buf());

    // Client route → index.html SPA shell
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let text = body_text(res).await;
    assert!(
        text.contains("tagw-spa"),
        "expected SPA index body, got: {text}"
    );

    // Nested client route also falls back
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/usage/detail")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_text(res).await.contains("tagw-spa"));

    // Real static asset
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/assets/app.js")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert!(body_text(res).await.contains("console.log('tagw')"));

    // API route still wins over SPA
    let res = app
        .oneshot(
            Request::builder()
                .uri("/healthz")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    assert_eq!(body_text(res).await, "ok");
}

#[tokio::test]
async fn build_app_without_static_does_not_serve_spa() {
    let state = AppState::new_for_test().await;
    let app = build_app(state);
    let res = app
        .oneshot(
            Request::builder()
                .uri("/dashboard")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn resolve_web_dir_defaults() {
    // Unset TAGW_WEB_DIR in this process may race if other tests set it; only
    // assert the constant and that resolve returns a path.
    assert_eq!(tagw::static_files::DEFAULT_WEB_DIR, "tagw/web/dist");
    let dir = tagw::static_files::resolve_web_dir();
    assert!(!dir.as_os_str().is_empty());
}
