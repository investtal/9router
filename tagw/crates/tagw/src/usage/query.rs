//! Read-side usage queries over `request_logs` (always time-bounded).

use chrono::{DateTime, Utc};
use rusqlite::{params, params_from_iter, OptionalExtension};
use serde::Serialize;

use crate::db::Db;
use crate::error::AppError;

/// Resolve range window start for overview aggregates (UTC).
pub fn range_start(range: &str, now: DateTime<Utc>) -> Result<DateTime<Utc>, AppError> {
    match range {
        "today" => Ok(now
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("midnight valid")
            .and_utc()),
        "3d" => Ok(now - chrono::Duration::days(3)),
        "7d" => Ok(now - chrono::Duration::days(7)),
        "30d" => Ok(now - chrono::Duration::days(30)),
        "90d" => Ok(now - chrono::Duration::days(90)),
        _ => Err(AppError::BadRequest(format!(
            "invalid range '{range}'; expected today|3d|7d|30d|90d"
        ))),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct UsageOverview {
    pub range: String,
    pub from: String,
    pub to: String,
    pub request_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub cost_est: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct RequestLogRow {
    pub id: String,
    pub created_at: String,
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

#[derive(Clone, Debug, Serialize)]
pub struct RequestListResponse {
    pub items: Vec<RequestLogRow>,
    /// Opaque cursor for next page (created_at of last row); null when no more.
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct RequestFilters {
    pub member_key_id: Option<String>,
    pub model: Option<String>,
    pub tool: Option<String>,
    pub status: Option<i32>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: u32,
    /// Cursor: exclusive upper bound on `created_at` (RFC3339) for keyset pagination.
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemberModelCell {
    pub member_key_id: String,
    pub member_name: Option<String>,
    pub model: String,
    pub request_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub cost_est: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemberUsageDetail {
    pub member_key_id: String,
    pub member_name: Option<String>,
    pub request_count: i64,
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    pub cached_tokens: i64,
    pub cost_est: f64,
    pub by_model: Vec<MemberModelCell>,
    pub recent: Vec<RequestLogRow>,
}

const DEFAULT_LIMIT: u32 = 50;
const MAX_LIMIT: u32 = 200;

pub fn clamp_limit(limit: Option<u32>) -> u32 {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// Aggregate totals for `range` ending at `now`.
pub fn query_overview(
    db: &Db,
    range: &str,
    now: DateTime<Utc>,
) -> Result<UsageOverview, AppError> {
    let from = range_start(range, now)?;
    let from_s = from.to_rfc3339();
    let to_s = now.to_rfc3339();

    let row = db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT
                    COUNT(*) AS request_count,
                    COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0),
                    COALESCE(SUM(cached_tokens), 0),
                    COALESCE(SUM(cost_est), 0.0)
                 FROM request_logs
                 WHERE created_at >= ?1 AND created_at <= ?2",
                params![from_s, to_s],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, f64>(4)?,
                    ))
                },
            )
        })
        .map_err(AppError::Internal)?;

    Ok(UsageOverview {
        range: range.to_string(),
        from: from_s,
        to: to_s,
        request_count: row.0,
        prompt_tokens: row.1,
        completion_tokens: row.2,
        cached_tokens: row.3,
        cost_est: row.4,
    })
}

/// Filtered request log listing (time-bounded via from/to or default 30d).
pub fn query_requests(db: &Db, filters: &RequestFilters) -> Result<RequestListResponse, AppError> {
    let now = Utc::now();
    let default_from = (now - chrono::Duration::days(30)).to_rfc3339();
    let from = filters
        .from
        .clone()
        .unwrap_or(default_from);
    let to = filters.to.clone().unwrap_or_else(|| now.to_rfc3339());
    let limit = filters.limit.max(1).min(MAX_LIMIT);

    let mut sql = String::from(
        "SELECT id, created_at, member_key_id, provider_id, account_id, model, tool, status,
                prompt_tokens, completion_tokens, cached_tokens, cost_est,
                latency_ms, ttft_ms, usage_incomplete, error
         FROM request_logs
         WHERE created_at >= ?1 AND created_at <= ?2",
    );
    let mut binds: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(from), Box::new(to)];

    if let Some(ref cursor) = filters.cursor {
        // Keyset: older than cursor (descending order).
        binds.push(Box::new(cursor.clone()));
        sql.push_str(&format!(" AND created_at < ?{}", binds.len()));
    }
    if let Some(ref mk) = filters.member_key_id {
        binds.push(Box::new(mk.clone()));
        sql.push_str(&format!(" AND member_key_id = ?{}", binds.len()));
    }
    if let Some(ref model) = filters.model {
        binds.push(Box::new(model.clone()));
        sql.push_str(&format!(" AND model = ?{}", binds.len()));
    }
    if let Some(ref tool) = filters.tool {
        binds.push(Box::new(tool.clone()));
        sql.push_str(&format!(" AND tool = ?{}", binds.len()));
    }
    if let Some(status) = filters.status {
        binds.push(Box::new(status));
        sql.push_str(&format!(" AND status = ?{}", binds.len()));
    }

    sql.push_str(" ORDER BY created_at DESC");
    binds.push(Box::new(i64::from(limit) + 1));
    sql.push_str(&format!(" LIMIT ?{}", binds.len()));

    let mut items = db
        .with_conn(|conn| {
            let mut stmt = conn.prepare(&sql)?;
            let params_iter = params_from_iter(binds.iter().map(|b| b.as_ref()));
            let rows = stmt.query_map(params_iter, map_request_row)?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
        .map_err(AppError::Internal)?;

    let next_cursor = if items.len() as u32 > limit {
        items.pop();
        items.last().map(|r| r.created_at.clone())
    } else {
        None
    };

    Ok(RequestListResponse {
        items,
        next_cursor,
    })
}

/// Member × model aggregate cells for the given range.
pub fn query_members(
    db: &Db,
    range: &str,
    now: DateTime<Utc>,
) -> Result<Vec<MemberModelCell>, AppError> {
    let from = range_start(range, now)?;
    let from_s = from.to_rfc3339();
    let to_s = now.to_rfc3339();

    db.with_conn(|conn| {
        let mut stmt = conn.prepare(
            "SELECT
                rl.member_key_id,
                mk.name,
                COALESCE(rl.model, '') AS model,
                COUNT(*) AS request_count,
                COALESCE(SUM(rl.prompt_tokens), 0),
                COALESCE(SUM(rl.completion_tokens), 0),
                COALESCE(SUM(rl.cached_tokens), 0),
                COALESCE(SUM(rl.cost_est), 0.0)
             FROM request_logs rl
             LEFT JOIN member_api_keys mk ON mk.id = rl.member_key_id
             WHERE rl.created_at >= ?1 AND rl.created_at <= ?2
               AND rl.member_key_id IS NOT NULL
             GROUP BY rl.member_key_id, mk.name, COALESCE(rl.model, '')
             ORDER BY request_count DESC",
        )?;
        let rows = stmt.query_map(params![from_s, to_s], |r| {
            Ok(MemberModelCell {
                member_key_id: r.get(0)?,
                member_name: r.get(1)?,
                model: r.get(2)?,
                request_count: r.get(3)?,
                prompt_tokens: r.get(4)?,
                completion_tokens: r.get(5)?,
                cached_tokens: r.get(6)?,
                cost_est: r.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    })
    .map_err(AppError::Internal)
}

/// Detail for one member key over the given range.
pub fn query_member_detail(
    db: &Db,
    key_id: &str,
    range: &str,
    now: DateTime<Utc>,
) -> Result<MemberUsageDetail, AppError> {
    let from = range_start(range, now)?;
    let from_s = from.to_rfc3339();
    let to_s = now.to_rfc3339();

    let member_name: Option<String> = db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT name FROM member_api_keys WHERE id = ?1",
                params![key_id],
                |r| r.get(0),
            )
            .optional()
        })
        .map_err(AppError::Internal)?;

    let (request_count, prompt_tokens, completion_tokens, cached_tokens, cost_est) = db
        .with_conn(|conn| {
            conn.query_row(
                "SELECT
                    COUNT(*),
                    COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0),
                    COALESCE(SUM(cached_tokens), 0),
                    COALESCE(SUM(cost_est), 0.0)
                 FROM request_logs
                 WHERE member_key_id = ?1
                   AND created_at >= ?2 AND created_at <= ?3",
                params![key_id, from_s, to_s],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, f64>(4)?,
                    ))
                },
            )
        })
        .map_err(AppError::Internal)?;

    let by_model = db
        .with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT
                    COALESCE(model, '') AS model,
                    COUNT(*),
                    COALESCE(SUM(prompt_tokens), 0),
                    COALESCE(SUM(completion_tokens), 0),
                    COALESCE(SUM(cached_tokens), 0),
                    COALESCE(SUM(cost_est), 0.0)
                 FROM request_logs
                 WHERE member_key_id = ?1
                   AND created_at >= ?2 AND created_at <= ?3
                 GROUP BY COALESCE(model, '')
                 ORDER BY COUNT(*) DESC",
            )?;
            let rows = stmt.query_map(params![key_id, from_s, to_s], |r| {
                Ok(MemberModelCell {
                    member_key_id: key_id.to_string(),
                    member_name: member_name.clone(),
                    model: r.get(0)?,
                    request_count: r.get(1)?,
                    prompt_tokens: r.get(2)?,
                    completion_tokens: r.get(3)?,
                    cached_tokens: r.get(4)?,
                    cost_est: r.get(5)?,
                })
            })?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row?);
            }
            Ok(out)
        })
        .map_err(AppError::Internal)?;

    let recent = query_requests(
        db,
        &RequestFilters {
            member_key_id: Some(key_id.to_string()),
            from: Some(from_s),
            to: Some(to_s),
            limit: 20,
            ..Default::default()
        },
    )?
    .items;

    Ok(MemberUsageDetail {
        member_key_id: key_id.to_string(),
        member_name,
        request_count,
        prompt_tokens,
        completion_tokens,
        cached_tokens,
        cost_est,
        by_model,
        recent,
    })
}

fn map_request_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<RequestLogRow> {
    Ok(RequestLogRow {
        id: r.get(0)?,
        created_at: r.get(1)?,
        member_key_id: r.get(2)?,
        provider_id: r.get(3)?,
        account_id: r.get(4)?,
        model: r.get(5)?,
        tool: r.get(6)?,
        status: r.get(7)?,
        prompt_tokens: r.get(8)?,
        completion_tokens: r.get(9)?,
        cached_tokens: r.get(10)?,
        cost_est: r.get(11)?,
        latency_ms: r.get(12)?,
        ttft_ms: r.get(13)?,
        usage_incomplete: r.get::<_, i64>(14)? != 0,
        error: r.get(15)?,
    })
}

/// Insert a request log row (tests / seeding).
pub fn insert_request_log(db: &Db, row: &RequestLogRow) -> Result<(), AppError> {
    db.with_conn(|conn| {
        conn.execute(
            "INSERT INTO request_logs (
                id, created_at, member_id, member_key_id, provider_id, account_id,
                model, tool, status, prompt_tokens, completion_tokens, cached_tokens,
                cost_est, latency_ms, ttft_ms, usage_incomplete, error
            ) VALUES (
                ?1, ?2, NULL, ?3, ?4, ?5,
                ?6, ?7, ?8, ?9, ?10, ?11,
                ?12, ?13, ?14, ?15, ?16
            )",
            params![
                row.id,
                row.created_at,
                row.member_key_id,
                row.provider_id,
                row.account_id,
                row.model,
                row.tool,
                row.status,
                row.prompt_tokens,
                row.completion_tokens,
                row.cached_tokens,
                row.cost_est,
                row.latency_ms,
                row.ttft_ms,
                i64::from(row.usage_incomplete),
                row.error,
            ],
        )?;
        Ok(())
    })
    .map_err(AppError::Internal)
}

#[cfg(test)]
mod unit_tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn range_start_today_and_days() {
        let now = Utc.with_ymd_and_hms(2026, 8, 3, 15, 30, 0).unwrap();
        let today = range_start("today", now).unwrap();
        assert_eq!(today.to_rfc3339(), "2026-08-03T00:00:00+00:00");
        assert_eq!(
            range_start("7d", now).unwrap(),
            now - chrono::Duration::days(7)
        );
        assert!(range_start("1y", now).is_err());
    }
}
