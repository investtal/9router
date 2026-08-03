#[derive(Clone, Debug)]
pub struct Config {
    pub bind: String,
    pub data_dir: std::path::PathBuf,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            bind: std::env::var("TAGW_BIND").unwrap_or_else(|_| "0.0.0.0:20128".into()),
            data_dir: std::env::var("TAGW_DATA_DIR")
                .map(Into::into)
                .unwrap_or_else(|_| "./data".into()),
        }
    }
}
