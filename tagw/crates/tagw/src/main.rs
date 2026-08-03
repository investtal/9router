use tagw::app::build_app;
use tagw::cache::ConfigCache;
use tagw::config::Config;
use tagw::db::Db;
use tagw::oauth::spawn_oauth_refresh_loop;
use tagw::state::AppState;
use tagw::usage::{spawn_usage_writer, USAGE_CHANNEL_CAPACITY};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cfg = Config::from_env();
    std::fs::create_dir_all(&cfg.data_dir)?;
    let db_path = cfg.data_dir.join("gateway.db");
    let db = Db::open(&db_path)?;
    db.migrate()?;
    let cache = ConfigCache::new();
    cache.load(&db)?;
    let (usage_tx, usage_rx) = tokio::sync::mpsc::channel(USAGE_CHANNEL_CAPACITY);
    let _usage_writer = spawn_usage_writer(db.clone(), usage_rx);
    let mut state = AppState::new(db, cache, usage_tx)
        .with_session_secret(cfg.session_secret.clone())
        .with_db_path(db_path);
    if let Some(base) = cfg.upstream.clone() {
        state = state.with_upstream(base, cfg.upstream_auth.clone());
        tracing::info!(upstream = %state.upstream_base.as_deref().unwrap_or(""), "proxy upstream configured");
    } else {
        tracing::warn!("TAGW_UPSTREAM not set; /v1 proxy will return 502 until configured");
    }
    if let Some(pub_base) = cfg.public_base.clone() {
        state = state.with_public_base(pub_base);
    }
    let _oauth_refresh = spawn_oauth_refresh_loop(
        state.db.clone(),
        state.cache.clone(),
        state.http_client.clone(),
    );
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("listening on {}", cfg.bind);
    axum::serve(listener, app).await?;
    Ok(())
}
