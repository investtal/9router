# Team AI Gateway Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use `subagent-parallel-execution` (recommended) or inline execution via `finishing-execution` to implement task-by-task. Steps use `- [ ]` checkboxes.

**Goal:** Ship a single-process Rust AI gateway + TanStack React dashboard that meets hop-latency SLOs under heavy concurrent load and covers the approved slim feature set (member keys, OAuth/API providers, RR+fail-over, usage/quota/logs, auth, import/export).

**Architecture:** One Rust binary (`tagw`) serves OpenAI-compatible + Anthropic proxy routes, control REST API, OAuth callbacks, SSE live logs, and static TanStack assets. SQLite WAL is source of truth; in-memory cache + async usage write queue keep the hot path off disk. Default HTTP stack is axum/hyper/tokio; Pingora is spike-gated via ADR after core works.

**Tech Stack:** Rust 1.85+, tokio, axum 0.8, hyper, tower, reqwest (streaming), rusqlite (bundled) or sqlx sqlite, argon2, jsonwebtoken/oauth2, serde, tracing; SPA: Vite + TanStack Router + React 19 + TypeScript; tests: cargo test + wiremock; optional Playwright later.

**Spec:** `docs/specs/2026-08-03-team-ai-gateway-design.md`

## Global Constraints

- Greenfield under `tagw/` — do **not** modify 9router Node hot path except docs/links.
- Latency: p95 hop &lt;20ms non-stream; p95 TTFB &lt;10ms stream; p99 TTFT add &lt;50ms; **zero full-body stream buffer**; stable 15+ parallel streams.
- Hot path must **never** `await` SQLite writes (usage via channel only).
- Member API keys: **argon2 hash** at rest; show prefix only.
- No encrypt-at-rest for provider secrets in v1 (admin-only APIs; redact logs).
- Auth default: basic username/password; OIDC Keycloak optional. Roles: `viewer` | `admin`.
- Routing: round-robin default; fail-over on 429 / selected 5xx / OAuth 401 after one refresh; no mid-body account switch after first response byte.
- Ranges: today, 3d, 7d, 30d, 90d.
- Out of scope: RTK, MITM, multi-tier product fallback, media/MCP/tunnels, multi-replica.
- TDD: failing test → implement → pass → commit per task.
- Execute in an isolated worktree via project git-worktree skill (never raw `git worktree add` without it).

### Access-pattern card (from spec — do not reinvent)

| Rank | Access | Representation |
|------|--------|----------------|
| 1 | key→member, account pool every LLM req | In-memory cache; SQLite SoT on mutate |
| 2 | append request log | mpsc + batch INSERT single writer |
| 3 | recent requests + filters | `request_logs` + indexes |
| 4–5 | aggregates / member×model | SQL GROUP BY on logs |
| 6 | export/import | DB file + JSON bundle |
| 7 | OAuth token update | row update + cache bust |

### Target tree

```text
tagw/
  Cargo.toml                 # workspace
  crates/tagw/
    Cargo.toml
    src/
      main.rs
      lib.rs
      config.rs
      error.rs
      state.rs
      db/{mod.rs,migrate.rs,schema.sql}
      cache/mod.rs
      auth/{mod.rs,member_key.rs,dashboard.rs,oidc.rs}
      router/account.rs
      proxy/{mod.rs,openai.rs,anthropic.rs,stream.rs}
      providers/{mod.rs,api_key.rs}
      oauth/{mod.rs,types.rs,refresh.rs,claude.rs,codex.rs,antigravity.rs,xai.rs,kimi.rs}
      usage/{mod.rs,writer.rs,query.rs,cost.rs}
      quota/mod.rs
      live/mod.rs
      admin/{mod.rs,providers.rs,keys.rs,users.rs,export.rs}
      static_files.rs
    tests/
      common/mod.rs
      proxy_stream.rs
      router_rr.rs
      authz.rs
      usage_query.rs
      import_export.rs
  scripts/slo_smoke.sh
  web/                       # TanStack SPA
    package.json
    vite.config.ts
    src/routes/...
docs/adr/2026-08-03-proxy-http-stack.md   # after spike
```

---

### Task 1: Scaffold workspace + health

**Files:**
- Create: `tagw/Cargo.toml`
- Create: `tagw/crates/tagw/Cargo.toml`
- Create: `tagw/crates/tagw/src/main.rs`
- Create: `tagw/crates/tagw/src/lib.rs`
- Create: `tagw/crates/tagw/src/config.rs`
- Create: `tagw/crates/tagw/src/error.rs`
- Create: `tagw/crates/tagw/src/state.rs`
- Test: `tagw/crates/tagw/tests/health.rs`

**Interfaces:**
- Consumes: none
- Produces: binary `tagw`; `AppState` placeholder; `GET /healthz` → 200 `ok`; `GET /readyz` → 200 when ready flag true

- [ ] **Step 1: Write the failing test**

```rust
// tagw/crates/tagw/tests/health.rs
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
```

- [ ] **Step 2: Run test, verify it fails** — Run: `cd tagw && cargo test -p tagw healthz_returns_ok`  
  Expected: FAIL (package/module missing)

- [ ] **Step 3: Write minimal implementation**

```toml
# tagw/Cargo.toml
[workspace]
members = ["crates/tagw"]
resolver = "2"
```

```toml
# tagw/crates/tagw/Cargo.toml
[package]
name = "tagw"
version = "0.1.0"
edition = "2021"

[dependencies]
axum = { version = "0.8", features = ["macros"] }
tokio = { version = "1", features = ["full"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["trace", "cors", "fs"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
thiserror = "2"
anyhow = "1"
uuid = { version = "1", features = ["v4", "serde"] }
chrono = { version = "0.4", features = ["serde"] }
```

```rust
// tagw/crates/tagw/src/lib.rs
pub mod app;
pub mod config;
pub mod error;
pub mod state;

pub use app::build_app;
```

```rust
// tagw/crates/tagw/src/config.rs
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
```

```rust
// tagw/crates/tagw/src/error.rs
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("unauthorized")]
    Unauthorized,
    #[error("forbidden")]
    Forbidden,
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("upstream: {0}")]
    Upstream(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match &self {
            AppError::Unauthorized => (StatusCode::UNAUTHORIZED, self.to_string()),
            AppError::Forbidden => (StatusCode::FORBIDDEN, self.to_string()),
            AppError::BadRequest(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            AppError::NotFound(_) => (StatusCode::NOT_FOUND, self.to_string()),
            AppError::Upstream(_) => (StatusCode::BAD_GATEWAY, self.to_string()),
            AppError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal error".into()),
        };
        (status, Json(json!({ "error": msg }))).into_response()
    }
}
```

```rust
// tagw/crates/tagw/src/state.rs
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub ready: Arc<AtomicBool>,
}

impl AppState {
    pub async fn new_for_test() -> Self {
        let s = Self {
            ready: Arc::new(AtomicBool::new(true)),
        };
        s
    }

    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
}
```

```rust
// tagw/crates/tagw/src/app.rs
use axum::{routing::get, Router};
use crate::state::AppState;

pub fn build_app(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route(
            "/readyz",
            get(|axum::extract::State(s): axum::extract::State<AppState>| async move {
                if s.is_ready() {
                    (axum::http::StatusCode::OK, "ready")
                } else {
                    (axum::http::StatusCode::SERVICE_UNAVAILABLE, "not ready")
                }
            }),
        )
        .with_state(state)
}
```

```rust
// tagw/crates/tagw/src/main.rs
use tagw::app::build_app;
use tagw::config::Config;
use tagw::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cfg = Config::from_env();
    let state = AppState::new_for_test().await;
    let app = build_app(state);
    let listener = tokio::net::TcpListener::bind(&cfg.bind).await?;
    tracing::info!("listening on {}", cfg.bind);
    axum::serve(listener, app).await?;
    Ok(())
}
```

- [ ] **Step 4: Run test, verify pass** — `cd tagw && cargo test -p tagw healthz_returns_ok`  
  Expected: PASS

- [ ] **Step 5: Commit** — `git add -f tagw && git commit -m "feat(tagw): scaffold rust binary and health endpoints"`

---

### Task 2: SQLite schema + migrations

**Files:**
- Create: `tagw/crates/tagw/src/db/mod.rs`
- Create: `tagw/crates/tagw/src/db/migrate.rs`
- Create: `tagw/crates/tagw/src/db/schema.sql`
- Modify: `tagw/crates/tagw/src/state.rs` (hold `Db`)
- Modify: `tagw/crates/tagw/Cargo.toml` (+ `rusqlite` with `bundled`, `r2d2`, `r2d2_sqlite` OR single `Mutex<Connection>` for writer + readers)
- Test: `tagw/crates/tagw/tests/migrate.rs`

**Interfaces:**
- Consumes: `Config.data_dir`
- Produces: `Db::open(path) -> Db`; `Db::migrate()`; tables per schema below

**Schema (`schema.sql`):**

```sql
PRAGMA journal_mode=WAL;
PRAGMA foreign_keys=ON;

CREATE TABLE IF NOT EXISTS schema_migrations (
  version INTEGER PRIMARY KEY,
  applied_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  username TEXT NOT NULL UNIQUE,
  password_hash TEXT,
  oidc_sub TEXT,
  role TEXT NOT NULL CHECK(role IN ('viewer','admin')),
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS member_api_keys (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  key_prefix TEXT NOT NULL,
  key_hash TEXT NOT NULL,
  created_at TEXT NOT NULL,
  revoked_at TEXT
);

CREATE TABLE IF NOT EXISTS providers (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL CHECK(kind IN ('oauth','api_key')),
  provider_type TEXT NOT NULL,
  name TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  config_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS accounts (
  id TEXT PRIMARY KEY,
  provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
  label TEXT NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1,
  credentials_json TEXT NOT NULL,
  quota_json TEXT NOT NULL DEFAULT '{}',
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS request_logs (
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL,
  member_id TEXT,
  member_key_id TEXT,
  provider_id TEXT,
  account_id TEXT,
  model TEXT,
  tool TEXT,
  status INTEGER,
  prompt_tokens INTEGER NOT NULL DEFAULT 0,
  completion_tokens INTEGER NOT NULL DEFAULT 0,
  cached_tokens INTEGER NOT NULL DEFAULT 0,
  cost_est REAL NOT NULL DEFAULT 0,
  latency_ms INTEGER,
  ttft_ms INTEGER,
  usage_incomplete INTEGER NOT NULL DEFAULT 0,
  error TEXT
);

CREATE INDEX IF NOT EXISTS idx_rl_created ON request_logs(created_at);
CREATE INDEX IF NOT EXISTS idx_rl_member_created ON request_logs(member_key_id, created_at);
CREATE INDEX IF NOT EXISTS idx_rl_model_created ON request_logs(model, created_at);
CREATE INDEX IF NOT EXISTS idx_rl_status_created ON request_logs(status, created_at);

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL
);
```

- [ ] **Step 1: Write failing test** — open temp dir, migrate, query `sqlite_master` for `request_logs`

```rust
#[tokio::test]
async fn migrate_creates_request_logs() {
    let dir = tempfile::tempdir().unwrap();
    let db = tagw::db::Db::open(dir.path().join("gateway.db")).unwrap();
    db.migrate().unwrap();
    let n: i64 = db
        .with_conn(|c| {
            c.query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name='request_logs'",
                [],
                |r| r.get(0),
            )
        })
        .unwrap();
    assert_eq!(n, 1);
}
```

- [ ] **Step 2: Run** — `cd tagw && cargo test -p tagw migrate_creates_request_logs`  
  Expected: FAIL

- [ ] **Step 3: Implement** `Db` with WAL, `migrate()` applying `schema.sql`, seed default admin if no users: username `admin` / password from `TAGW_ADMIN_PASSWORD` (default `admin` for dev only — log warning).

- [ ] **Step 4: Pass** — same cargo test PASS

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): sqlite WAL schema and migrations"`

---

### Task 3: Config cache + member API keys (hash + auth)

**Files:**
- Create: `tagw/crates/tagw/src/cache/mod.rs`
- Create: `tagw/crates/tagw/src/auth/mod.rs`
- Create: `tagw/crates/tagw/src/auth/member_key.rs`
- Create: `tagw/crates/tagw/src/admin/keys.rs`
- Modify: `state.rs`, `app.rs`
- Test: `tagw/crates/tagw/tests/member_key_auth.rs`

**Interfaces:**
- Consumes: `Db`
- Produces:
  - `create_member_key(name) -> (MemberApiKeyRow, plaintext_once)`
  - `ConfigCache::authenticate_bearer(token: &str) -> Option<MemberContext>`
  - `MemberContext { key_id: String, name: String }`
  - Admin routes: `POST /api/admin/keys`, `GET /api/admin/keys`, `DELETE /api/admin/keys/:id` (auth stub: allow all until Task 10 — mark `// AUTHZ: Task 10`)

```rust
// auth/member_key.rs — core API
pub struct MemberContext {
    pub key_id: String,
    pub name: String,
}

pub fn hash_key(plaintext: &str) -> String { /* argon2 */ }
pub fn verify_key(plaintext: &str, hash: &str) -> bool { /* argon2 */ }
pub fn generate_key() -> (String /* full sk-... */, String /* prefix 8 chars */) { }
```

- [ ] **Step 1: Failing test**

```rust
#[tokio::test]
async fn created_key_authenticates_and_revoked_does_not() {
    // open db, migrate, create key, cache.load(), authenticate ok
    // revoke, cache.reload(), authenticate None
}
```

- [ ] **Step 2: cargo test — FAIL**

- [ ] **Step 3: Implement** argon2 hashing, `ConfigCache` with `DashMap` or `RwLock<HashMap>` for prefix→candidates then verify hash; reload on mutate.

- [ ] **Step 4: PASS**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): member api keys with argon2 and config cache"`

---

### Task 4: Usage write queue (non-blocking)

**Files:**
- Create: `tagw/crates/tagw/src/usage/mod.rs`
- Create: `tagw/crates/tagw/src/usage/writer.rs`
- Create: `tagw/crates/tagw/src/usage/cost.rs`
- Modify: `state.rs` (hold `UsageTx`)
- Test: `tagw/crates/tagw/tests/usage_writer.rs`

**Interfaces:**
- Produces:

```rust
#[derive(Clone, Debug)]
pub struct UsageEvent {
    pub id: String,
    pub created_at: String, // RFC3339
    pub member_key_id: Option<String>,
    pub provider_id: Option<String>,
    pub account_id: Option<String>,
    pub model: Option<String>,
    pub tool: Option<String>,
    pub status: Option<i32>,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub cost_est: f64,
    pub latency_ms: Option<i64>,
    pub ttft_ms: Option<i64>,
    pub usage_incomplete: bool,
    pub error: Option<String>,
}

pub type UsageTx = tokio::sync::mpsc::Sender<UsageEvent>;
pub fn spawn_usage_writer(db: Db, rx: mpsc::Receiver<UsageEvent>) -> JoinHandle<()>;
// batch every 50ms or 64 events; single connection writes
```

- [ ] **Step 1: Test** — send 100 events, sleep, count rows == 100; assert send is non-blocking (buffer capacity ≥ 1024)

- [ ] **Step 2–4: TDD implement + pass**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): async batched usage writer"`

---

### Task 5: Streaming proxy core (OpenAI-compatible) + zero full-body buffer

**Files:**
- Create: `tagw/crates/tagw/src/proxy/mod.rs`
- Create: `tagw/crates/tagw/src/proxy/stream.rs`
- Create: `tagw/crates/tagw/src/proxy/openai.rs`
- Modify: `app.rs` routes `/{*path}` under `/v1`
- Add deps: `reqwest` with `stream`, `bytes`, `http-body-util`, `futures-util`
- Test: `tagw/crates/tagw/tests/proxy_stream.rs`

**Interfaces:**
- Consumes: `MemberKeyAuth`, `UsageTx`, temporary single upstream URL from env `TAGW_UPSTREAM` for this task (account router in Task 6)
- Produces: `POST /v1/chat/completions` streaming passthrough

**Critical stream helper:**

```rust
// proxy/stream.rs
/// Pipe upstream bytes to client without collecting the full body.
pub fn forward_byte_stream<S>(stream: S) -> axum::body::Body
where
    S: futures_util::Stream<Item = Result<bytes::Bytes, reqwest::Error>> + Send + 'static,
{
    use futures_util::StreamExt;
    Body::from_stream(stream.map(|r| r.map_err(|e| std::io::Error::other(e))))
}
```

- [ ] **Step 1: Failing integration test with wiremock**

```rust
#[tokio::test]
async fn chat_completions_streams_chunks_without_buffering_all() {
    // Mock upstream SSE that sends 3 chunks with delays
    // Client with member key hits gateway
    // Assert status 200, body contains all 3 chunks in order
    // Assert mock received Authorization from upstream config
}
```

Also unit-ish: document that `bytes_stream()` is used, never `response.bytes().await` for stream path.

- [ ] **Step 2: FAIL**

- [ ] **Step 3: Implement**
  - Extract bearer → member
  - Forward method/path/query/headers (strip hop-by-hop)
  - Stream body both ways
  - On complete: enqueue `UsageEvent` (tokens best-effort parse from stream lines if `usage` appears; else `usage_incomplete=true`)
  - Measure `latency_ms` and `ttft_ms` (first byte time)

- [ ] **Step 4: PASS**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): openai-compatible streaming proxy passthrough"`

---

### Task 6: AccountRouter — round-robin + fail-over

**Files:**
- Create: `tagw/crates/tagw/src/router/account.rs`
- Modify: `cache/mod.rs` to hold pools
- Modify: `proxy/openai.rs` to use router
- Test: `tagw/crates/tagw/tests/router_rr.rs`

**Interfaces:**

```rust
pub struct AccountRef {
    pub account_id: String,
    pub provider_id: String,
    pub upstream_base: String,
    pub auth_header: String, // "Bearer ..." or raw
}

pub struct AccountRouter { /* cursors: Mutex<HashMap<String /*pool*/, usize>> */ }

impl AccountRouter {
    pub fn pick(&self, pool_key: &str, accounts: &[AccountRef]) -> Option<AccountRef>;
    pub fn should_failover(status: u16) -> bool {
        matches!(status, 429 | 500 | 502 | 503 | 504)
    }
}

pub const MAX_FAILOVER_ATTEMPTS: usize = 3;
```

**Fail-over rules (tests must encode):**
1. RR advances cursor each successful pick
2. Disabled accounts skipped
3. On fail-over status before first response byte → try next account up to `MAX_FAILOVER_ATTEMPTS`
4. After first byte forwarded → no switch; finish/error stream

- [ ] **Step 1: Unit tests for pick order + skip disabled + failover helper**

- [ ] **Step 2–4: Implement + integration with two mock upstreams (first 429, second 200)**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): round-robin account router with fail-over"`

---

### Task 7: API-key providers CRUD + wire into pools

**Files:**
- Create: `tagw/crates/tagw/src/providers/mod.rs`
- Create: `tagw/crates/tagw/src/providers/api_key.rs`
- Create: `tagw/crates/tagw/src/admin/providers.rs`
- Modify: cache reload from DB
- Test: `tagw/crates/tagw/tests/api_key_provider.rs`

**Provider types (enum string):**  
`glm`, `open_model`, `alibaba`, `anthropic`, `minimax`, `kimi`, `deepseek` (+ generic `openai_compat` base URL)

**Admin API:**
- `GET/POST /api/admin/providers`
- `POST /api/admin/providers/:id/accounts` body `{ label, api_key, base_url?, models? }`
- `PATCH` enable/disable

- [ ] **Step 1: Test** create provider+account → cache has pool → proxy uses that base URL (wiremock)

- [ ] **Step 2–4: Implement**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): api-key providers and admin CRUD"`

---

### Task 8: OAuth manager + token refresh (Codex first, then others)

**Files:**
- Create: `tagw/crates/tagw/src/oauth/mod.rs`
- Create: `tagw/crates/tagw/src/oauth/types.rs`
- Create: `tagw/crates/tagw/src/oauth/refresh.rs`
- Create: `tagw/crates/tagw/src/oauth/codex.rs`
- Create: `tagw/crates/tagw/src/oauth/claude.rs`
- Create: `tagw/crates/tagw/src/oauth/antigravity.rs`
- Create: `tagw/crates/tagw/src/oauth/xai.rs`
- Create: `tagw/crates/tagw/src/oauth/kimi.rs`
- Routes: `GET /api/oauth/:provider/start`, `GET /api/oauth/:provider/callback`
- Test: `tagw/crates/tagw/tests/oauth_refresh.rs`

**Interfaces:**

```rust
#[async_trait::async_trait]
pub trait OAuthProvider: Send + Sync {
    fn id(&self) -> &'static str;
    async fn exchange_code(&self, code: &str, pkce: &Pkce) -> anyhow::Result<TokenSet>;
    async fn refresh(&self, refresh_token: &str) -> anyhow::Result<TokenSet>;
}

pub struct TokenSet {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Ensure valid access token; refresh if expires within 120s or on force.
pub async fn ensure_access_token(db: &Db, cache: &ConfigCache, account_id: &str) -> Result<String, AppError>;
```

**Background:** tokio task every 60s scans accounts with `expires_at < now+5min` and refreshes.

- [ ] **Step 1: Test** mock refresh endpoint; expired token → `ensure_access_token` calls refresh once → new token stored

- [ ] **Step 2–4:** Implement Codex fully with tests; stub other providers with same trait (compile + `todo` only if start URL unknown — **prefer** implementing connect flows from 9router reference `src/lib/oauth/services/*` behavior, porting endpoints carefully)

**Reference (read-only):**  
`src/lib/oauth/services/codex.js`, `claude.js`, `antigravity.js`, `xai.js`, and Kimi service files under 9router for endpoint URLs and PKCE patterns. Port constants; do not call Node.

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): oauth connect and auto token refresh"`

---

### Task 9: Anthropic Messages path (Claude Code)

**Files:**
- Create: `tagw/crates/tagw/src/proxy/anthropic.rs`
- Modify: `app.rs` — `POST /v1/messages`, `POST /v1/messages/count_tokens` if needed
- Test: `tagw/crates/tagw/tests/anthropic_stream.rs`

**Behavior:**
- Accept Anthropic `x-api-key` **or** Bearer member key (member key preferred for team attribution)
- Forward to selected Anthropic-compatible account stream
- Same fail-over + usage rules as OpenAI path

- [ ] **Step 1: wiremock Anthropic SSE fixture → gateway streams through**

- [ ] **Step 2–4: Implement**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): anthropic messages streaming for claude code"`

---

### Task 10: Dashboard auth (basic) + RBAC

**Files:**
- Create: `tagw/crates/tagw/src/auth/dashboard.rs`
- Create: `tagw/crates/tagw/src/admin/users.rs`
- Modify: all `/api/admin/*` and `/api/*` dashboard routes with layers
- Test: `tagw/crates/tagw/tests/authz.rs`

**Interfaces:**

```rust
pub enum Role { Viewer, Admin }

pub struct DashboardUser {
    pub id: String,
    pub username: String,
    pub role: Role,
}

// Session: signed cookie `tagw_session` with user id (use `axum-extra` cookies + HMAC from TAGW_SESSION_SECRET)
pub async fn login_basic(username, password) -> Result<Session, AppError>;
pub fn require_role(user: &DashboardUser, min: Role) -> Result<(), AppError>;
```

**Rules:**
- `GET /api/usage/*`, `GET /api/providers` (redacted), `GET /api/logs/*` → any authenticated
- Mutate keys/providers/export/users → **admin only**
- LLM `/v1/*` → member key only (unchanged)

- [ ] **Step 1: Tests** viewer 403 on POST keys; admin 200; unauthenticated 401

- [ ] **Step 2–4: Implement**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): basic dashboard auth and viewer/admin rbac"`

---

### Task 11: OIDC (Keycloak)

**Files:**
- Create: `tagw/crates/tagw/src/auth/oidc.rs`
- Settings keys: `oidc.enabled`, `oidc.issuer`, `oidc.client_id`, `oidc.client_secret`, `oidc.redirect_uri`
- Routes: `/api/auth/oidc/start`, `/api/auth/oidc/callback`
- Test: `tagw/crates/tagw/tests/oidc_login.rs` with mock JWKS/token endpoint

- [ ] **Step 1: Test** code exchange mock → session created; role map default `viewer` unless claim `realm_access.roles` contains `tagw-admin`

- [ ] **Step 2–4: Implement** (basic still works when OIDC disabled)

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): optional keycloak oidc dashboard login"`

---

### Task 12: Usage query API (ranges + filters + members)

**Files:**
- Create: `tagw/crates/tagw/src/usage/query.rs`
- Create: `tagw/crates/tagw/src/admin/usage_routes.rs` (or `api/usage.rs`)
- Test: `tagw/crates/tagw/tests/usage_query.rs`

**Endpoints:**

| Method | Path | Notes |
|--------|------|-------|
| GET | `/api/usage/overview` | `range=today\|3d\|7d\|30d\|90d` → totals |
| GET | `/api/usage/requests` | filters: member_key_id, model, tool, status, from, to, limit, cursor |
| GET | `/api/usage/members` | member × model cells |
| GET | `/api/usage/members/:key_id` | one member detail |

**Range helper:**

```rust
pub fn range_start(range: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, AppError> {
    match range {
        "today" => /* local or UTC midnight — use UTC for v1 */ Ok(now.date_naive().and_hms_opt(0,0,0).unwrap().and_utc()),
        "3d" => Ok(now - chrono::Duration::days(3)),
        "7d" => Ok(now - chrono::Duration::days(7)),
        "30d" => Ok(now - chrono::Duration::days(30)),
        "90d" => Ok(now - chrono::Duration::days(90)),
        _ => Err(AppError::BadRequest("invalid range".into())),
    }
}
```

Overview SQL aggregates: `COUNT(*)`, `SUM(prompt_tokens)`, `SUM(cached_tokens)`, `SUM(completion_tokens)`, `SUM(cost_est)`.

- [ ] **Step 1: Seed logs; assert overview + member filters**

- [ ] **Step 2–4: Implement with indexes used (no full table scan without time bound)**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): usage overview members and request filters"`

---

### Task 13: Live console SSE + recent ring buffer

**Files:**
- Create: `tagw/crates/tagw/src/live/mod.rs`
- Modify: proxy to `live.publish(event)`
- Route: `GET /api/logs/stream`
- Test: `tagw/crates/tagw/tests/live_sse.rs`

**Interfaces:**

```rust
pub struct LiveEvent {
    pub id: String,
    pub ts: String,
    pub level: String, // info|warn|error
    pub message: String,
    pub request_id: Option<String>,
    pub member_key_id: Option<String>,
    pub model: Option<String>,
}

pub struct LiveLogHub {
    // broadcast::Sender<LiveEvent>
    // ring: Mutex<VecDeque<LiveEvent>> capacity 500
}
```

- [ ] **Step 1: Subscribe SSE, publish event, assert client receives JSON line**

- [ ] **Step 2–4: Implement**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): realtime console log sse"`

---

### Task 14: Quota tracker views

**Files:**
- Create: `tagw/crates/tagw/src/quota/mod.rs`
- Route: `GET /api/quota`
- Test: `tagw/crates/tagw/tests/quota.rs`

**Behavior:**
- Read `accounts.quota_json` (provider-populated when available)
- Merge derived usage from `request_logs` for last 30d per account
- OAuth providers: refresh quota snapshot hooks where APIs exist; else show derived only + `source: derived|provider`

- [ ] **Step 1–4: TDD**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): quota tracker for oauth and api-key accounts"`

---

### Task 15: Import / export

**Files:**
- Create: `tagw/crates/tagw/src/admin/export.rs`
- Routes:
  - `GET /api/admin/export/db` → download `gateway.db` (admin)
  - `GET /api/admin/export/bundle` → JSON bundle
  - `POST /api/admin/import/bundle` → multipart/json
- Test: `tagw/crates/tagw/tests/import_export.rs`

**Bundle schema:**

```json
{
  "version": 1,
  "exported_at": "...",
  "providers": [],
  "accounts": [],
  "users": [],
  "member_api_keys": [{ "id", "name", "key_prefix", "key_hash", "created_at", "revoked_at" }],
  "settings": {},
  "include_request_logs": false
}
```

**Policy:** default bundle includes key **hashes** only (no plaintext keys). DB file download is full fidelity for trusted VPS copy.

- [ ] **Step 1: export → wipe → import → provider count and key auth still work with same plaintext if re-imported hash matches**

- [ ] **Step 2–4: Implement transactional import (reject invalid → no partial)**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): database and json bundle import export"`

---

### Task 16: Serve static TanStack SPA + CORS

**Files:**
- Create: `tagw/crates/tagw/src/static_files.rs`
- Modify: `main.rs` — fallback to `index.html` for non-API routes
- Env: `TAGW_WEB_DIR` default `tagw/web/dist`

- [ ] **Step 1: Test** that unknown `/dashboard` path returns index when file exists (temp dist fixture)

- [ ] **Step 2–4: Implement `ServeDir` + SPA fallback; API routes take precedence**

- [ ] **Step 5: Commit** — `git commit -m "feat(tagw): serve tanstack static assets from rust binary"`

---

### Task 17: TanStack React dashboard (core pages)

**Files:**
- Create: `tagw/web/package.json`
- Create: `tagw/web/vite.config.ts` (proxy `/api` and `/v1` to `http://127.0.0.1:20128` in dev)
- Create: `tagw/web/src/main.tsx`
- Create: `tagw/web/src/routes/__root.tsx`
- Create: `tagw/web/src/routes/login.tsx`
- Create: `tagw/web/src/routes/index.tsx` (overview)
- Create: `tagw/web/src/routes/usage.tsx`
- Create: `tagw/web/src/routes/members.tsx`
- Create: `tagw/web/src/routes/providers.tsx`
- Create: `tagw/web/src/routes/logs.tsx`
- Create: `tagw/web/src/routes/admin.keys.tsx`
- Create: `tagw/web/src/lib/api.ts`
- Test: `tagw/web` — `npm test` or vitest for `range` query builder; manual checklist acceptable for UI if vitest unit only

**Stack pin:**
- `@tanstack/react-router`, `react` 19, `vite` 6+, typescript

**`api.ts`:**

```ts
export type Range = 'today' | '3d' | '7d' | '30d' | '90d';

export async function fetchOverview(range: Range) {
  const r = await fetch(`/api/usage/overview?range=${range}`, { credentials: 'include' });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
}
```

- [ ] **Step 1: Scaffold with `npm create` / manual package.json; vitest for api helper**

- [ ] **Step 2: Implement login + overview numbers + providers list + live logs EventSource**

- [ ] **Step 3: `npm run build` produces `dist/`; rust serves it**

- [ ] **Step 4: Commit** — `git commit -m "feat(tagw-web): tanstack dashboard core pages"`

---

### Task 18: SLO smoke script + CI test job

**Files:**
- Create: `tagw/scripts/slo_smoke.sh`
- Create: `tagw/scripts/mock_upstream.py` or rust mock binary
- Optional: `.github/workflows/tagw.yml`

**Script requirements:**
1. Start mock upstream that streams SSE with controlled delay  
2. Start `tagw` pointed at mock  
3. Fire 50 concurrent stream requests  
4. Measure client TTFB; assert p95 &lt; 10ms **added** vs direct-to-mock baseline (same machine)  
5. Exit non-zero on failure  

- [ ] **Step 1: Implement script; run locally**  
  Expected: PASS on quiet machine (document flaky CI note)

- [ ] **Step 2: Commit** — `git commit -m "test(tagw): concurrent stream slo smoke script"`

---

### Task 19: Pingora vs axum spike + ADR

**Files:**
- Create: `tagw/spike/README.md` (how to run)
- Create: `docs/adr/2026-08-03-proxy-http-stack.md` (date adjust on write)
- Optional spike crate under `tagw/spike/`

**ADR must include:**

| Metric | axum/hyper | Pingora |
|--------|------------|---------|
| TTFB p95 | measured | measured |
| TTFB p99 | measured | measured |
| CPU @ 50 streams | | |
| Decision | **stay axum** or **switch** | reason |

Default decision if spike skipped for time: **stay axum** with justification "admin+proxy cohesion; SLOs met on axum smoke".

- [ ] **Step 1: Run spike or write ADR with axum stay + smoke evidence from Task 18**

- [ ] **Step 2: Commit** — `git commit -m "docs(adr): proxy http stack axum vs pingora decision"`

---

### Task 20: End-to-end hardening checklist (manual + fix gaps)

**Files:** modify as needed from gaps

**Checklist (all must be true before "v1 done"):**
- [ ] Codex pointed at `http://GATEWAY/v1` with member key works
- [ ] pi same
- [ ] Claude Code Anthropic base URL or OpenAI mode works
- [ ] RR across 2 API keys for one provider
- [ ] Kill one upstream (429) → fail-over
- [ ] OAuth refresh survives process restart (tokens in SQLite)
- [ ] Viewer cannot create keys; admin can
- [ ] Export DB, wipe data dir, restore, usage history present
- [ ] Live logs show requests within 1s
- [ ] No secrets in `tracing` output (grep test)

- [ ] **Step 1: Run checklist; file bugs as code fixes in this task**

- [ ] **Step 2: Commit** — `git commit -m "fix(tagw): e2e hardening from v1 checklist"`

---

## Spec coverage map

| Spec requirement | Task(s) |
|------------------|---------|
| Manage secrets / member keys | 3, 7, 10 |
| OAuth providers + refresh | 8 |
| API-key providers | 7 |
| Usage overview + filters + members | 4, 12 |
| Quota tracker | 14 |
| Realtime console + recent | 12, 13 |
| Import/export | 15 |
| Basic + OIDC auth | 10, 11 |
| Round-robin + fail-over | 6 |
| Viewer/admin roles | 10 |
| Latency SLOs / no full buffer | 5, 18 |
| Claude Code / Codex / pi | 5, 9, 20 |
| Pingora evaluation | 19 |
| Single Rust binary + TanStack | 1, 16, 17 |
| SQLite WAL + write queue | 2, 4 |

---

## Self-review notes

1. **Spec coverage:** mapped above; no orphan requirements.  
2. **Placeholders:** OAuth provider endpoint constants must be filled from 9router reference during Task 8 — implementer reads those files; plan points to exact paths.  
3. **Type consistency:** `UsageEvent`, `MemberContext`, `AccountRef`, `Role` names stable across tasks.  
4. **Data-first:** schema Task 2 after access card; writer Task 4 before heavy proxy; indexes in schema for query Task 12.  
5. **Authz timing:** Task 7 marks admin routes open until Task 10 — intentional; Task 10 closes the hole (tests in 10 must prove 403).

---

## Execution notes

- Default bind port **20128** (same as 9router) for drop-in client config — change via `TAGW_BIND` if both run.  
- Do not delete 9router until Task 20 signed off.  
- Force-add under `docs/` if gitignored; `tagw/` should be normal tracked paths (ensure not ignored).

---

*End of implementation plan.*
