use tagw::app::build_app;
use tagw::config::Config;
use tagw::db::Db;
use tagw::state::AppState;

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
    let state = AppState::new(db);
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("listening on {}", cfg.bind);
    axum::serve(listener, app).await?;
    Ok(())
}
