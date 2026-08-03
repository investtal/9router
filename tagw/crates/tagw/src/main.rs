use tagw::app::build_app;
use tagw::cache::ConfigCache;
use tagw::config::Config;
use tagw::db::Db;
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
    let state = AppState::new(db, cache, usage_tx);
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("listening on {}", cfg.bind);
    axum::serve(listener, app).await?;
    Ok(())
}
