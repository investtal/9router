//! Usage overview / request list / member breakdown APIs (AuthUser).

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use chrono::Utc;
use serde::Deserialize;

use crate::auth::dashboard::AuthUser;
use crate::error::AppError;
use crate::state::AppState;
use crate::usage::query::{
    clamp_limit, query_member_detail, query_members, query_overview, query_request_by_id,
    query_requests, RequestFilters,
};

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/usage/overview", get(usage_overview))
        .route("/api/usage/requests", get(usage_requests))
        .route("/api/usage/requests/{id}", get(usage_request_detail))
        .route("/api/usage/members", get(usage_members))
        .route("/api/usage/members/{key_id}", get(usage_member_detail))
}

#[derive(Debug, Deserialize)]
pub struct OverviewQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

fn default_range() -> String {
    "7d".into()
}

async fn usage_overview(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<OverviewQuery>,
) -> Result<Json<crate::usage::query::UsageOverview>, AppError> {
    let overview = query_overview(&state.db, &q.range, Utc::now())?;
    Ok(Json(overview))
}

#[derive(Debug, Deserialize)]
pub struct RequestsQuery {
    pub member_key_id: Option<String>,
    pub model: Option<String>,
    pub tool: Option<String>,
    pub status: Option<i32>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<u32>,
    pub cursor: Option<String>,
}

async fn usage_requests(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<RequestsQuery>,
) -> Result<Json<crate::usage::query::RequestListResponse>, AppError> {
    let filters = RequestFilters {
        member_key_id: q.member_key_id,
        model: q.model,
        tool: q.tool,
        status: q.status,
        from: q.from,
        to: q.to,
        limit: clamp_limit(q.limit),
        cursor: q.cursor,
    };
    let list = query_requests(&state.db, &filters)?;
    Ok(Json(list))
}

async fn usage_request_detail(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(id): Path<String>,
) -> Result<Json<crate::usage::query::RequestLogRow>, AppError> {
    query_request_by_id(&state.db, &id)?
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("request '{id}' not found")))
}

#[derive(Debug, Deserialize)]
pub struct MembersQuery {
    #[serde(default = "default_range")]
    pub range: String,
}

async fn usage_members(
    State(state): State<AppState>,
    _user: AuthUser,
    Query(q): Query<MembersQuery>,
) -> Result<Json<Vec<crate::usage::query::MemberModelCell>>, AppError> {
    let cells = query_members(&state.db, &q.range, Utc::now())?;
    Ok(Json(cells))
}

async fn usage_member_detail(
    State(state): State<AppState>,
    _user: AuthUser,
    Path(key_id): Path<String>,
    Query(q): Query<MembersQuery>,
) -> Result<Json<crate::usage::query::MemberUsageDetail>, AppError> {
    let detail = query_member_detail(&state.db, &key_id, &q.range, Utc::now())?;
    Ok(Json(detail))
}
