//! Bundle export → wipe → import round-trip; invalid import is rejected.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tagw::admin::export::{build_bundle, import_bundle, ExportBundle, BUNDLE_VERSION};
use tagw::app::build_app;
use tagw::auth::member_key::create_member_key;
use tagw::providers::api_key::{
    create_account, create_provider, CreateAccountRequest, CreateProviderRequest,
};
use tagw::state::AppState;
use tower::ServiceExt;

async fn read_json(res: axum::response::Response) -> Value {
    let body = axum::body::to_bytes(res.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&body).unwrap_or_else(|_| {
        panic!(
            "expected json, got: {}",
            String::from_utf8_lossy(&body)
        )
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn export_wipe_import_restores_providers_and_key_auth() {
    let state = AppState::new_for_test().await;

    let prov = create_provider(
        &state.db,
        &CreateProviderRequest {
            provider_type: "deepseek".into(),
            name: "DeepSeek".into(),
            enabled: Some(true),
            config_json: None,
        },
    )
    .unwrap();
    let _acct = create_account(
        &state.db,
        &prov.id,
        &CreateAccountRequest {
            label: "primary".into(),
            api_key: "sk-deepseek-secret".into(),
            base_url: None,
            models: None,
            enabled: Some(true),
        },
    )
    .unwrap();
    let (key_row, plaintext) = create_member_key(&state.db, "team-key").unwrap();
    state.cache.reload(&state.db).unwrap();
    assert!(
        state.cache.authenticate_bearer(&plaintext).is_some(),
        "plaintext must auth before export"
    );

    // Seed a setting
    state
        .db
        .with_conn(|c| {
            c.execute(
                "INSERT INTO settings (key, value_json) VALUES ('demo.flag', 'true')
                 ON CONFLICT(key) DO UPDATE SET value_json = excluded.value_json",
                [],
            )
        })
        .unwrap();

    let app = build_app(state.clone());
    let cookie = state.test_session_cookie("admin");

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/export/bundle")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let exported = read_json(res).await;
    assert_eq!(exported["version"], BUNDLE_VERSION);
    assert!(exported["providers"].as_array().unwrap().len() >= 1);
    assert!(exported["accounts"].as_array().unwrap().len() >= 1);
    // Hashes only — no plaintext key field
    let keys = exported["member_api_keys"].as_array().unwrap();
    assert!(keys.iter().any(|k| k["id"] == key_row.id));
    assert!(keys.iter().all(|k| k.get("key").is_none()));
    assert!(keys.iter().any(|k| k["key_hash"].as_str().unwrap().contains("argon2")));

    // Wipe live data
    state
        .db
        .with_conn(|c| {
            c.execute_batch(
                "DELETE FROM request_logs;
                 DELETE FROM accounts;
                 DELETE FROM providers;
                 DELETE FROM member_api_keys;
                 DELETE FROM settings;",
            )
        })
        .unwrap();
    state.cache.reload(&state.db).unwrap();
    assert!(state.cache.authenticate_bearer(&plaintext).is_none());
    let n_prov: i64 = state
        .db
        .with_conn(|c| c.query_row("SELECT COUNT(*) FROM providers", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(n_prov, 0);

    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/admin/import/bundle")
                .header("cookie", &cookie)
                .header("content-type", "application/json")
                .body(Body::from(exported.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "import body: {:?}", res.status());
    let result = read_json(res).await;
    assert!(result["providers"].as_u64().unwrap() >= 1);
    assert!(result["member_api_keys"].as_u64().unwrap() >= 1);

    let n_prov: i64 = state
        .db
        .with_conn(|c| c.query_row("SELECT COUNT(*) FROM providers", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(n_prov, exported["providers"].as_array().unwrap().len() as i64);

    // Same plaintext still authenticates after hash re-import + cache reload
    assert!(
        state.cache.authenticate_bearer(&plaintext).is_some(),
        "imported key_hash must verify original plaintext"
    );

    // Settings restored
    let flag: String = state
        .db
        .with_conn(|c| {
            c.query_row(
                "SELECT value_json FROM settings WHERE key = 'demo.flag'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(flag, "true");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invalid_bundle_version_rejects_without_partial_write() {
    let state = AppState::new_for_test().await;
    let before: i64 = state
        .db
        .with_conn(|c| c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)))
        .unwrap();
    assert!(before >= 1, "seeded admin");

    let mut bundle = build_bundle(&state.db, false).unwrap();
    bundle.version = 99;
    let err = import_bundle(&state.db, &bundle).unwrap_err();
    assert!(err.to_string().contains("version"), "{err}");

    let after: i64 = state
        .db
        .with_conn(|c| c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(before, after, "reject must not wipe users");

    // Missing provider ref
    let mut bad = build_bundle(&state.db, false).unwrap();
    if let Some(a) = bad.accounts.first_mut() {
        a.provider_id = "missing-provider".into();
    } else {
        bad.accounts.push(tagw::admin::export::AccountBundle {
            id: "a1".into(),
            provider_id: "missing-provider".into(),
            label: "x".into(),
            enabled: true,
            credentials_json: json!({}),
            quota_json: json!({}),
            created_at: "2026-01-01T00:00:00Z".into(),
        });
    }
    let err = import_bundle(&state.db, &bad).unwrap_err();
    assert!(err.to_string().contains("missing provider"), "{err}");
    let after2: i64 = state
        .db
        .with_conn(|c| c.query_row("SELECT COUNT(*) FROM users", [], |r| r.get(0)))
        .unwrap();
    assert_eq!(before, after2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn export_db_requires_path_and_admin() {
    let state = AppState::new_for_test().await;
    let app = build_app(state.clone());

    // No path configured
    let cookie = state.test_session_cookie("admin");
    let res = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/api/admin/export/db")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);

    // Unauthenticated
    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/export/bundle")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn export_db_downloads_file_when_path_set() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("gateway.db");
    let db = tagw::db::Db::open(&db_path).unwrap();
    db.migrate().unwrap();
    let cache = tagw::cache::ConfigCache::new();
    cache.load(&db).unwrap();
    let (usage_tx, usage_rx) = tokio::sync::mpsc::channel(tagw::usage::USAGE_CHANNEL_CAPACITY);
    let _w = tagw::usage::spawn_usage_writer(db.clone(), usage_rx);
    let state = AppState::new(db, cache, usage_tx)
        .with_session_secret(tagw::auth::dashboard::DEFAULT_SESSION_SECRET)
        .with_db_path(&db_path);
    let cookie = state.test_session_cookie("admin");
    let app = build_app(state);

    let res = app
        .oneshot(
            Request::builder()
                .uri("/api/admin/export/db")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let ct = res
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(ct, "application/octet-stream");
    let body = axum::body::to_bytes(res.into_body(), 10 * 1024 * 1024)
        .await
        .unwrap();
    assert!(body.len() > 100, "sqlite file should be non-trivial");
    // SQLite magic header
    assert_eq!(&body[..16], b"SQLite format 3\0");
}

// Silence unused import warning if ExportBundle only used via type in one path
#[allow(dead_code)]
fn _bundle_type_check(b: ExportBundle) -> u32 {
    b.version
}
