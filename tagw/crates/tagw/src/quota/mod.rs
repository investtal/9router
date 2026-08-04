//! Quota tracker: merge provider `accounts.quota_json` with derived usage.

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use chrono::{Duration, Utc};
use rusqlite::params;
use serde::Serialize;
use serde_json::{json, Value};

use crate::auth::dashboard::AuthUser;
use crate::db::Db;
use crate::error::AppError;
use crate::state::AppState;

/// Per-account quota view for the dashboard.
#[derive(Clone, Debug, Serialize)]
pub struct AccountQuota {
    pub account_id: String,
    pub provider_id: String,
    pub provider_type: String,
    pub provider_kind: String,
    pub label: String,
    pub enabled: bool,
    /// `provider` when `quota_json` has meaningful keys; else `derived`.
    pub source: String,
    /// Raw provider snapshot (may be `{}`).
    pub quota_json: Value,
    /// Derived from `request_logs` for the last 30 days.
    pub derived: DerivedUsage,
}

#[derive(Clone, Debug, Serialize)]
pub struct DerivedUsage {
    pub window_days: u32,
    pub from: String,
    pub to: String,
    pub request_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub cost_est: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct QuotaResponse {
    pub accounts: Vec<AccountQuota>,
}

/// True when provider-populated quota looks non-empty (not `{}` / null).
pub fn quota_source(quota_json: &Value) -> &'static str {
    match quota_json {
        Value::Object(map) if !map.is_empty() => "provider",
        Value::Array(arr) if !arr.is_empty() => "provider",
        Value::String(s) if !s.is_empty() && s != "{}" => "provider",
        Value::Number(_) | Value::Bool(_) => "provider",
        _ => "derived",
    }
}

/// Load all accounts with merged quota + 30d derived usage.
pub fn list_account_quotas(db: &Db) -> Result<Vec<AccountQuota>, AppError> {
    let now = Utc::now();
    let from = now - Duration::days(30);
    let from_s = from.to_rfc3339();
    let to_s = now.to_rfc3339();

    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT a.id, a.provider_id, p.provider_type, p.kind, a.label, a.enabled, a.quota_json
             FROM accounts a
             JOIN providers p ON p.id = a.provider_id
             ORDER BY p.name, a.label",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)? != 0,
                r.get::<_, String>(6)?,
            ))
        })?;

        let mut accounts = Vec::new();
        for row in rows {
            let (account_id, provider_id, provider_type, provider_kind, label, enabled, quota_raw) =
                row?;
            let quota_json: Value =
                serde_json::from_str(&quota_raw).unwrap_or_else(|_| json!({}));
            let source = quota_source(&quota_json).to_string();

            let derived = conn.query_row(
                "SELECT
                    COUNT(*),
                    COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0),
                    COALESCE(SUM(cached_tokens), 0),
                    COALESCE(SUM(cost_est), 0.0)
                 FROM request_logs
                 WHERE account_id = ?1
                   AND created_at >= ?2 AND created_at <= ?3",
                params![account_id, from_s, to_s],
                |r| {
                    Ok(DerivedUsage {
                        window_days: 30,
                        from: from_s.clone(),
                        to: to_s.clone(),
                        request_count: r.get(0)?,
                        prompt_tokens: r.get(1)?,
                        completion_tokens: r.get(2)?,
                        cached_tokens: r.get(3)?,
                        cost_est: r.get(4)?,
                    })
                },
            )?;

            accounts.push(AccountQuota {
                account_id,
                provider_id,
                provider_type,
                provider_kind,
                label,
                enabled,
                source,
                quota_json,
                derived,
            });
        }
        Ok(accounts)
    })
    .map_err(AppError::Internal)
}

/// Optional hook: refresh provider quota snapshots when APIs exist.
/// v1 is a no-op stub; derived usage always remains available.
pub async fn refresh_provider_quotas_if_available(_state: &AppState) {
    // Future: Claude/Codex/etc. usage endpoints when credentials support them.
}

pub fn router() -> Router<AppState> {
    Router::new().route("/api/quota", get(get_quota))
}

async fn get_quota(
    State(state): State<AppState>,
    _user: AuthUser,
) -> Result<Json<QuotaResponse>, AppError> {
    refresh_provider_quotas_if_available(&state).await;
    let accounts = list_account_quotas(&state.db)?;
    Ok(Json(QuotaResponse { accounts }))
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn source_empty_is_derived() {
        assert_eq!(quota_source(&json!({})), "derived");
        assert_eq!(quota_source(&Value::Null), "derived");
    }

    #[test]
    fn source_nonempty_is_provider() {
        assert_eq!(quota_source(&json!({"remaining": 100})), "provider");
        assert_eq!(quota_source(&json!([1])), "provider");
    }
}
