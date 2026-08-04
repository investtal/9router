//! Serve the TanStack SPA (`TAGW_WEB_DIR`, default `tagw/web/dist`).
//!
//! Uses `tower_http::ServeDir` with SPA fallback to `index.html` for client routes.
//! Mount via [`with_static_files`] as a router fallback so `/api/*`, `/v1/*`, and
//! health routes registered earlier take precedence.

use std::path::PathBuf;

use axum::http::{header, HeaderValue, Method};
use axum::Router;
use tower_http::cors::{AllowOrigin, CorsLayer};
use tower_http::services::{ServeDir, ServeFile};

/// Default web asset directory when `TAGW_WEB_DIR` is unset.
pub const DEFAULT_WEB_DIR: &str = "tagw/web/dist";

/// Resolve web dir from env (`TAGW_WEB_DIR`) or [`DEFAULT_WEB_DIR`].
pub fn resolve_web_dir() -> PathBuf {
    std::env::var("TAGW_WEB_DIR")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_WEB_DIR))
}

/// CORS for same-origin SPA + Vite dev server (`localhost:5173`).
pub fn cors_layer() -> CorsLayer {
    let origins = [
        "http://127.0.0.1:5173",
        "http://localhost:5173",
        "http://127.0.0.1:20128",
        "http://localhost:20128",
    ]
    .into_iter()
    .filter_map(|o| o.parse::<HeaderValue>().ok())
    .collect::<Vec<_>>();

    CorsLayer::new()
        .allow_origin(AllowOrigin::list(origins))
        .allow_credentials(true)
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
            header::COOKIE,
        ])
}

/// Attach SPA static file serving as the router's fallback service.
///
/// API and proxy routes already registered on `router` win; everything else is
/// served from `web_dir`, with missing paths falling back to `index.html`.
pub fn with_static_files<S>(router: Router<S>, web_dir: PathBuf) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    if !web_dir.exists() {
        tracing::warn!(
            path = %web_dir.display(),
            "TAGW web dir missing — SPA will 404 until assets are built"
        );
    } else {
        tracing::info!(path = %web_dir.display(), "serving TanStack SPA");
    }
    // Use `.fallback` (not `.not_found_service`) so SPA client routes return 200
    // with index.html rather than 404 + body.
    let index = web_dir.join("index.html");
    let serve = ServeDir::new(&web_dir)
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(index));
    router.fallback_service(serve)
}
