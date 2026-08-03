//! Admin import/export: full DB file download + portable JSON bundle.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use rusqlite::params;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::auth::dashboard::AdminUser;
use crate::db::Db;
use crate::error::AppError;
use crate::state::AppState;

/// Bundle format version (reject other versions on import).
pub const BUNDLE_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExportBundle {
    pub version: u32,
    pub exported_at: String,
    pub providers: Vec<ProviderBundle>,
    pub accounts: Vec<AccountBundle>,
    pub users: Vec<UserBundle>,
    pub member_api_keys: Vec<MemberKeyBundle>,
    pub settings: Value,
    #[serde(default)]
    pub include_request_logs: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub request_logs: Vec<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderBundle {
    pub id: String,
    pub kind: String,
    pub provider_type: String,
    pub name: String,
    pub enabled: bool,
    pub config_json: Value,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AccountBundle {
    pub id: String,
    pub provider_id: String,
    pub label: String,
    pub enabled: bool,
    /// Full credentials for restore (not redacted in bundle).
    pub credentials_json: Value,
    pub quota_json: Value,
    pub created_at: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UserBundle {
    pub id: String,
    pub username: String,
    pub password_hash: Option<String>,
    pub oidc_sub: Option<String>,
    pub role: String,
    pub created_at: String,
}

/// Member keys with **hashes only** (no plaintext secrets).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MemberKeyBundle {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub key_hash: String,
    pub created_at: String,
    pub revoked_at: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImportResult {
    pub providers: usize,
    pub accounts: usize,
    pub users: usize,
    pub member_api_keys: usize,
    pub settings: usize,
    pub request_logs: usize,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/admin/export/db", get(export_db))
        .route("/api/admin/export/bundle", get(export_bundle_handler))
        .route("/api/admin/import/bundle", post(import_bundle_handler))
}

async fn export_db(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Response, AppError> {
    let path = state.db_path.as_ref().ok_or_else(|| {
        AppError::BadRequest("database path not configured (db export unavailable)".into())
    })?;

    // Checkpoint WAL so the main file is a consistent snapshot.
    state
        .db
        .with_conn(|conn| {
            let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            Ok(())
        })
        .map_err(AppError::Internal)?;

    let bytes = std::fs::read(path).map_err(|e| {
        AppError::Internal(anyhow::anyhow!("read database file {}: {e}", path.display()))
    })?;

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            "attachment; filename=\"gateway.db\"",
        )
        .body(Body::from(bytes))
        .map_err(|e| AppError::Internal(anyhow::anyhow!("build response: {e}")))
}

async fn export_bundle_handler(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<ExportBundle>, AppError> {
    let bundle = build_bundle(&state.db, false)?;
    Ok(Json(bundle))
}

async fn import_bundle_handler(
    State(state): State<AppState>,
    _admin: AdminUser,
    Json(bundle): Json<ExportBundle>,
) -> Result<Json<ImportResult>, AppError> {
    let result = import_bundle(&state.db, &bundle)?;
    if let Err(e) = state.cache.reload(&state.db) {
        tracing::warn!(error = %e, "config cache reload after import failed");
    }
    Ok(Json(result))
}

/// Build a portable JSON bundle from the live DB.
pub fn build_bundle(db: &Db, include_request_logs: bool) -> Result<ExportBundle, AppError> {
    db.with_conn(|conn| {
        let mut providers = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT id, kind, provider_type, name, enabled, config_json, created_at
                 FROM providers ORDER BY created_at",
            )?;
            let rows = stmt.query_map([], |r| {
                let config_raw: String = r.get(5)?;
                Ok(ProviderBundle {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    provider_type: r.get(2)?,
                    name: r.get(3)?,
                    enabled: r.get::<_, i64>(4)? != 0,
                    config_json: serde_json::from_str(&config_raw)
                        .unwrap_or_else(|_| Value::Object(Default::default())),
                    created_at: r.get(6)?,
                })
            })?;
            for row in rows {
                providers.push(row?);
            }
        }

        let mut accounts = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT id, provider_id, label, enabled, credentials_json, quota_json, created_at
                 FROM accounts ORDER BY created_at",
            )?;
            let rows = stmt.query_map([], |r| {
                let cred_raw: String = r.get(4)?;
                let quota_raw: String = r.get(5)?;
                Ok(AccountBundle {
                    id: r.get(0)?,
                    provider_id: r.get(1)?,
                    label: r.get(2)?,
                    enabled: r.get::<_, i64>(3)? != 0,
                    credentials_json: serde_json::from_str(&cred_raw)
                        .unwrap_or_else(|_| Value::Object(Default::default())),
                    quota_json: serde_json::from_str(&quota_raw)
                        .unwrap_or_else(|_| Value::Object(Default::default())),
                    created_at: r.get(6)?,
                })
            })?;
            for row in rows {
                accounts.push(row?);
            }
        }

        let mut users = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT id, username, password_hash, oidc_sub, role, created_at
                 FROM users ORDER BY created_at",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(UserBundle {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password_hash: r.get(2)?,
                    oidc_sub: r.get(3)?,
                    role: r.get(4)?,
                    created_at: r.get(5)?,
                })
            })?;
            for row in rows {
                users.push(row?);
            }
        }

        let mut member_api_keys = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT id, name, key_prefix, key_hash, created_at, revoked_at
                 FROM member_api_keys ORDER BY created_at",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(MemberKeyBundle {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    key_prefix: r.get(2)?,
                    key_hash: r.get(3)?,
                    created_at: r.get(4)?,
                    revoked_at: r.get(5)?,
                })
            })?;
            for row in rows {
                member_api_keys.push(row?);
            }
        }

        let mut settings_map = serde_json::Map::new();
        {
            let mut stmt = conn.prepare("SELECT key, value_json FROM settings")?;
            let rows = stmt.query_map([], |r| {
                let k: String = r.get(0)?;
                let v: String = r.get(1)?;
                Ok((k, v))
            })?;
            for row in rows {
                let (k, v) = row?;
                let parsed: Value =
                    serde_json::from_str(&v).unwrap_or(Value::String(v));
                settings_map.insert(k, parsed);
            }
        }

        let mut request_logs = Vec::new();
        if include_request_logs {
            let mut stmt = conn.prepare(
                "SELECT id, created_at, member_id, member_key_id, provider_id, account_id,
                        model, tool, status, prompt_tokens, completion_tokens, cached_tokens,
                        cost_est, latency_ms, ttft_ms, usage_incomplete, error
                 FROM request_logs ORDER BY created_at",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, String>(0)?,
                    "created_at": r.get::<_, String>(1)?,
                    "member_id": r.get::<_, Option<String>>(2)?,
                    "member_key_id": r.get::<_, Option<String>>(3)?,
                    "provider_id": r.get::<_, Option<String>>(4)?,
                    "account_id": r.get::<_, Option<String>>(5)?,
                    "model": r.get::<_, Option<String>>(6)?,
                    "tool": r.get::<_, Option<String>>(7)?,
                    "status": r.get::<_, Option<i32>>(8)?,
                    "prompt_tokens": r.get::<_, i64>(9)?,
                    "completion_tokens": r.get::<_, i64>(10)?,
                    "cached_tokens": r.get::<_, i64>(11)?,
                    "cost_est": r.get::<_, f64>(12)?,
                    "latency_ms": r.get::<_, Option<i64>>(13)?,
                    "ttft_ms": r.get::<_, Option<i64>>(14)?,
                    "usage_incomplete": r.get::<_, i64>(15)? != 0,
                    "error": r.get::<_, Option<String>>(16)?,
                }))
            })?;
            for row in rows {
                request_logs.push(row?);
            }
        }

        Ok(ExportBundle {
            version: BUNDLE_VERSION,
            exported_at: Utc::now().to_rfc3339(),
            providers,
            accounts,
            users,
            member_api_keys,
            settings: Value::Object(settings_map),
            include_request_logs,
            request_logs,
        })
    })
    .map_err(AppError::Internal)
}

/// Transactional import. Invalid version / validation errors leave DB unchanged.
pub fn import_bundle(db: &Db, bundle: &ExportBundle) -> Result<ImportResult, AppError> {
    if bundle.version != BUNDLE_VERSION {
        return Err(AppError::BadRequest(format!(
            "unsupported bundle version {}; expected {BUNDLE_VERSION}",
            bundle.version
        )));
    }

    // Pre-validate roles / kinds before opening the write transaction.
    for u in &bundle.users {
        if u.role != "viewer" && u.role != "admin" {
            return Err(AppError::BadRequest(format!(
                "invalid user role '{}'",
                u.role
            )));
        }
        if u.username.trim().is_empty() {
            return Err(AppError::BadRequest("user username must not be empty".into()));
        }
    }
    for p in &bundle.providers {
        if p.kind != "oauth" && p.kind != "api_key" {
            return Err(AppError::BadRequest(format!(
                "invalid provider kind '{}'",
                p.kind
            )));
        }
        if p.id.trim().is_empty() || p.name.trim().is_empty() {
            return Err(AppError::BadRequest(
                "provider id and name must not be empty".into(),
            ));
        }
    }
    let provider_ids: std::collections::HashSet<&str> =
        bundle.providers.iter().map(|p| p.id.as_str()).collect();
    for a in &bundle.accounts {
        if !provider_ids.contains(a.provider_id.as_str()) {
            return Err(AppError::BadRequest(format!(
                "account '{}' references missing provider '{}'",
                a.id, a.provider_id
            )));
        }
    }
    for k in &bundle.member_api_keys {
        if k.key_hash.trim().is_empty() || k.key_prefix.trim().is_empty() {
            return Err(AppError::BadRequest(
                "member_api_keys require key_hash and key_prefix".into(),
            ));
        }
    }

    let settings_entries: Vec<(String, String)> = match &bundle.settings {
        Value::Object(m) => m
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    serde_json::to_string(v).unwrap_or_else(|_| "null".into()),
                )
            })
            .collect(),
        Value::Null => Vec::new(),
        other => {
            return Err(AppError::BadRequest(format!(
                "settings must be a JSON object, got {other}"
            )));
        }
    };

    db.with_conn(|conn| {
        let tx = conn.unchecked_transaction()?;

        // FK-safe wipe of importable tables (schema_migrations stays).
        tx.execute_batch(
            "DELETE FROM request_logs;
             DELETE FROM accounts;
             DELETE FROM providers;
             DELETE FROM member_api_keys;
             DELETE FROM users;
             DELETE FROM settings;",
        )?;

        for p in &bundle.providers {
            let config_str = serde_json::to_string(&p.config_json).unwrap_or_else(|_| "{}".into());
            tx.execute(
                "INSERT INTO providers (id, kind, provider_type, name, enabled, config_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    p.id,
                    p.kind,
                    p.provider_type,
                    p.name,
                    if p.enabled { 1 } else { 0 },
                    config_str,
                    p.created_at,
                ],
            )?;
        }

        for a in &bundle.accounts {
            let cred = serde_json::to_string(&a.credentials_json).unwrap_or_else(|_| "{}".into());
            let quota = serde_json::to_string(&a.quota_json).unwrap_or_else(|_| "{}".into());
            tx.execute(
                "INSERT INTO accounts (id, provider_id, label, enabled, credentials_json, quota_json, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    a.id,
                    a.provider_id,
                    a.label,
                    if a.enabled { 1 } else { 0 },
                    cred,
                    quota,
                    a.created_at,
                ],
            )?;
        }

        for u in &bundle.users {
            tx.execute(
                "INSERT INTO users (id, username, password_hash, oidc_sub, role, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    u.id,
                    u.username,
                    u.password_hash,
                    u.oidc_sub,
                    u.role,
                    u.created_at,
                ],
            )?;
        }

        for k in &bundle.member_api_keys {
            tx.execute(
                "INSERT INTO member_api_keys (id, name, key_prefix, key_hash, created_at, revoked_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    k.id,
                    k.name,
                    k.key_prefix,
                    k.key_hash,
                    k.created_at,
                    k.revoked_at,
                ],
            )?;
        }

        for (key, value_json) in &settings_entries {
            tx.execute(
                "INSERT INTO settings (key, value_json) VALUES (?1, ?2)",
                params![key, value_json],
            )?;
        }

        let mut request_logs_n = 0usize;
        if bundle.include_request_logs {
            for log in &bundle.request_logs {
                let id = match log.get("id").and_then(|v| v.as_str()) {
                    Some(id) => id,
                    None => {
                        return Err(rusqlite::Error::ToSqlConversionFailure(Box::new(
                            std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "request_logs entry missing id",
                            ),
                        )));
                    }
                };
                let created_at = log
                    .get("created_at")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                tx.execute(
                    "INSERT INTO request_logs (
                        id, created_at, member_id, member_key_id, provider_id, account_id,
                        model, tool, status, prompt_tokens, completion_tokens, cached_tokens,
                        cost_est, latency_ms, ttft_ms, usage_incomplete, error
                    ) VALUES (
                        ?1, ?2, ?3, ?4, ?5, ?6,
                        ?7, ?8, ?9, ?10, ?11, ?12,
                        ?13, ?14, ?15, ?16, ?17
                    )",
                    params![
                        id,
                        created_at,
                        log.get("member_id").and_then(|v| v.as_str()),
                        log.get("member_key_id").and_then(|v| v.as_str()),
                        log.get("provider_id").and_then(|v| v.as_str()),
                        log.get("account_id").and_then(|v| v.as_str()),
                        log.get("model").and_then(|v| v.as_str()),
                        log.get("tool").and_then(|v| v.as_str()),
                        log.get("status").and_then(|v| v.as_i64()).map(|n| n as i32),
                        log.get("prompt_tokens").and_then(|v| v.as_i64()).unwrap_or(0),
                        log.get("completion_tokens")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0),
                        log.get("cached_tokens").and_then(|v| v.as_i64()).unwrap_or(0),
                        log.get("cost_est").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        log.get("latency_ms").and_then(|v| v.as_i64()),
                        log.get("ttft_ms").and_then(|v| v.as_i64()),
                        i64::from(
                            log.get("usage_incomplete")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                        ),
                        log.get("error").and_then(|v| v.as_str()),
                    ],
                )?;
                request_logs_n += 1;
            }
        }

        tx.commit()?;
        Ok(ImportResult {
            providers: bundle.providers.len(),
            accounts: bundle.accounts.len(),
            users: bundle.users.len(),
            member_api_keys: bundle.member_api_keys.len(),
            settings: settings_entries.len(),
            request_logs: request_logs_n,
        })
    })
    .map_err(|e| {
        let msg = e.to_string();
        if msg.contains("CHECK")
            || msg.contains("UNIQUE")
            || msg.contains("FOREIGN")
            || msg.contains("request_logs entry missing")
        {
            AppError::BadRequest(format!("import rejected: {msg}"))
        } else {
            AppError::Internal(e)
        }
    })
}
