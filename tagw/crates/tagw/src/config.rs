#[derive(Clone, Debug)]
pub struct Config {
    pub bind: String,
    pub data_dir: std::path::PathBuf,
    /// Temporary single upstream base URL (Task 5). Replaced by AccountRouter in Task 6.
    /// Example: `http://127.0.0.1:8080` — request path `/v1/...` is appended.
    pub upstream: Option<String>,
    /// Optional `Authorization` header value sent to the upstream (e.g. `Bearer sk-...`).
    pub upstream_auth: Option<String>,
    /// Public base URL for OAuth `redirect_uri` construction (e.g. `http://127.0.0.1:20128`).
    pub public_base: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            bind: std::env::var("TAGW_BIND").unwrap_or_else(|_| "0.0.0.0:20128".into()),
            data_dir: std::env::var("TAGW_DATA_DIR")
                .map(Into::into)
                .unwrap_or_else(|_| "./data".into()),
            upstream: std::env::var("TAGW_UPSTREAM").ok().filter(|s| !s.is_empty()),
            upstream_auth: std::env::var("TAGW_UPSTREAM_AUTH")
                .ok()
                .filter(|s| !s.is_empty()),
            public_base: std::env::var("TAGW_PUBLIC_BASE")
                .ok()
                .filter(|s| !s.is_empty()),
        }
    }
}
