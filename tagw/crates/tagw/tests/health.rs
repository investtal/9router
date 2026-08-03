use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt;
use tagw::app::build_app;
use tagw::state::AppState;

#[tokio::test]
async fn healthz_returns_ok() {
    let state = AppState::new_for_test().await;
    let app = build_app(state);
    let res = app
        .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
}
