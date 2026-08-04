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
  error TEXT,
  -- Client request / upstream response payloads (truncated; 9router-style detail).
  request_body TEXT,
  response_body TEXT
);

CREATE INDEX IF NOT EXISTS idx_rl_created ON request_logs(created_at);
CREATE INDEX IF NOT EXISTS idx_rl_member_created ON request_logs(member_key_id, created_at);
CREATE INDEX IF NOT EXISTS idx_rl_model_created ON request_logs(model, created_at);
CREATE INDEX IF NOT EXISTS idx_rl_status_created ON request_logs(status, created_at);

CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value_json TEXT NOT NULL
);
