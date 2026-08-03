//! Anthropic Messages API path for Claude Code (`POST /v1/messages`).
//!
//! Auth: Bearer member key **preferred**, or Anthropic-style `x-api-key` with the
//! same member key. Upstream credentials come from AccountRouter (prefer
//! [`crate::state::ANTHROPIC_POOL_KEY`], else default pool, else `TAGW_UPSTREAM`).
//!
//! Same fail-over + OAuth ensure + UsageEvent rules as the OpenAI path.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode};
use axum::response::Response;
use bytes::Bytes;

use crate::auth::member_key::MemberContext;
use crate::error::AppError;
use crate::oauth::ensure_access_token_with_client;
use crate::proxy::stream::forward_io_stream;
use crate::router::{AccountRef, AccountRouter, MAX_FAILOVER_ATTEMPTS};
use crate::state::{AppState, ANTHROPIC_POOL_KEY, DEFAULT_POOL_KEY};
use crate::usage::{estimate_cost, UsageEvent};

const LINE_BUF_MAX: usize = 256 * 1024;
const MAX_REQUEST_BODY: usize = 32 * 1024 * 1024;

const DEFAULT_ANTHROPIC_VERSION: &str = "2023-06-01";

fn is_hop_by_hop(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailers"
            | "transfer-encoding"
            | "upgrade"
            | "host"
            | "content-length"
    )
}

fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let val = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = val
        .strip_prefix("Bearer ")
        .or_else(|| val.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

fn extract_x_api_key(headers: &HeaderMap) -> Option<&str> {
    let val = headers.get("x-api-key")?.to_str().ok()?;
    let val = val.trim();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

/// Authenticate: prefer valid Bearer member key; else treat `x-api-key` as member key.
fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<MemberContext, AppError> {
    if let Some(token) = extract_bearer(headers) {
        return state
            .cache
            .authenticate_bearer(token)
            .ok_or(AppError::Unauthorized);
    }
    if let Some(key) = extract_x_api_key(headers) {
        return state
            .cache
            .authenticate_bearer(key)
            .ok_or(AppError::Unauthorized);
    }
    Err(AppError::Unauthorized)
}

/// Parse Anthropic SSE / JSON usage (input_tokens / output_tokens / cache_*).
fn parse_usage_from_line(line: &str, metrics: &mut StreamMetrics) {
    let line = line.trim();
    if line.is_empty() || line.starts_with("event:") {
        return;
    }
    let json_str = line.strip_prefix("data:").map(str::trim).unwrap_or(line);
    if json_str.is_empty() || json_str == "[DONE]" {
        return;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return;
    };

    // message_start nests model + usage under `message`.
    let message = v.get("message");
    if let Some(model) = v
        .get("model")
        .or_else(|| message.and_then(|m| m.get("model")))
        .and_then(|m| m.as_str())
    {
        if metrics.model.is_none() {
            metrics.model = Some(model.to_string());
        }
    }

    let usage = v
        .get("usage")
        .or_else(|| message.and_then(|m| m.get("usage")));
    let Some(usage) = usage else {
        return;
    };
    if usage.is_null() {
        return;
    }

    let prompt = usage
        .get("input_tokens")
        .or_else(|| usage.get("prompt_tokens"))
        .and_then(|x| x.as_i64());
    let completion = usage
        .get("output_tokens")
        .or_else(|| usage.get("completion_tokens"))
        .and_then(|x| x.as_i64());
    let cache_read = usage
        .get("cache_read_input_tokens")
        .or_else(|| usage.get("cached_tokens"))
        .and_then(|x| x.as_i64())
        .unwrap_or(0);
    let cache_create = usage
        .get("cache_creation_input_tokens")
        .and_then(|x| x.as_i64())
        .unwrap_or(0);

    if let Some(p) = prompt {
        metrics.prompt_tokens = p;
    }
    if let Some(c) = completion {
        // message_delta often carries only output_tokens (cumulative).
        metrics.completion_tokens = c;
    }
    metrics.cached_tokens = cache_read + cache_create;
    if prompt.is_some() || completion.is_some() {
        metrics.usage_incomplete = false;
    }
}

#[derive(Debug)]
struct StreamMetrics {
    start: Instant,
    first_byte_at: Option<Instant>,
    member_key_id: String,
    status: u16,
    model: Option<String>,
    prompt_tokens: i64,
    completion_tokens: i64,
    cached_tokens: i64,
    usage_incomplete: bool,
    error: Option<String>,
    line_buf: String,
    provider_id: Option<String>,
    account_id: Option<String>,
}

impl StreamMetrics {
    fn new(member_key_id: String, status: u16, start: Instant) -> Self {
        Self {
            start,
            first_byte_at: None,
            member_key_id,
            status,
            model: None,
            prompt_tokens: 0,
            completion_tokens: 0,
            cached_tokens: 0,
            usage_incomplete: true,
            error: None,
            line_buf: String::new(),
            provider_id: None,
            account_id: None,
        }
    }

    fn with_account(mut self, account: &AccountRef) -> Self {
        self.provider_id = Some(account.provider_id.clone());
        self.account_id = Some(account.account_id.clone());
        self
    }

    fn on_bytes(&mut self, chunk: &Bytes) {
        if self.first_byte_at.is_none() && !chunk.is_empty() {
            self.first_byte_at = Some(Instant::now());
        }
        if let Ok(s) = std::str::from_utf8(chunk) {
            self.line_buf.push_str(s);
            self.drain_complete_lines();
            if self.line_buf.len() > LINE_BUF_MAX {
                tracing::warn!(
                    len = self.line_buf.len(),
                    cap = LINE_BUF_MAX,
                    "anthropic usage line_buf exceeded cap; clearing"
                );
                self.line_buf.clear();
                self.usage_incomplete = true;
            }
        }
    }

    fn drain_complete_lines(&mut self) {
        while let Some(pos) = self.line_buf.find('\n') {
            let mut line = self.line_buf.drain(..=pos).collect::<String>();
            if line.ends_with('\n') {
                line.pop();
            }
            if line.ends_with('\r') {
                line.pop();
            }
            parse_usage_from_line(&line, self);
        }
    }

    fn flush_line_buf(&mut self) {
        if self.line_buf.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.line_buf);
        let line = line.trim_end_matches(['\r', '\n']);
        if !line.is_empty() {
            parse_usage_from_line(line, self);
        }
    }

    fn into_event(mut self) -> UsageEvent {
        self.flush_line_buf();
        let latency_ms = Some(self.start.elapsed().as_millis() as i64);
        let ttft_ms = self
            .first_byte_at
            .map(|t| t.duration_since(self.start).as_millis() as i64);
        let cost_est = estimate_cost(
            self.model.as_deref(),
            self.prompt_tokens,
            self.completion_tokens,
            self.cached_tokens,
        );
        UsageEvent {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            member_key_id: Some(self.member_key_id),
            provider_id: self.provider_id,
            account_id: self.account_id,
            model: self.model,
            tool: Some("anthropic".into()),
            status: Some(self.status as i32),
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            cached_tokens: self.cached_tokens,
            cost_est,
            latency_ms,
            ttft_ms,
            usage_incomplete: self.usage_incomplete,
            error: self.error,
        }
    }
}

struct UsageOnComplete {
    metrics: Arc<Mutex<Option<StreamMetrics>>>,
    usage_tx: crate::usage::UsageTx,
    live: crate::live::LiveLogHub,
}

impl Drop for UsageOnComplete {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.metrics.lock() {
            if let Some(m) = guard.take() {
                let ev = m.into_event();
                self.live.publish(crate::live::request_complete_event(
                    ev.id.clone(),
                    ev.member_key_id.clone(),
                    ev.model.clone(),
                    ev.status,
                    ev.error.as_deref(),
                ));
                if let Err(e) = self.usage_tx.try_send(ev) {
                    tracing::warn!(
                        error = %e,
                        "usage channel full or closed; dropping Anthropic UsageEvent"
                    );
                }
            }
        }
    }
}

/// Strip client auth; inject account credentials for Anthropic-compatible upstreams.
///
/// - OAuth accounts: `Authorization: Bearer <token>`
/// - API-key accounts: `x-api-key: <key>` (from `Bearer <key>` or raw)
/// - Fallback env auth: `Bearer …` → Authorization; otherwise x-api-key
fn build_upstream_headers(
    req_headers: &HeaderMap,
    account: Option<&AccountRef>,
    fallback_auth: Option<&str>,
) -> Result<HeaderMap, AppError> {
    let mut out = HeaderMap::new();
    for (name, value) in req_headers.iter() {
        if is_hop_by_hop(name.as_str()) {
            continue;
        }
        // Never forward client auth upstream.
        if name == header::AUTHORIZATION || name.as_str().eq_ignore_ascii_case("x-api-key") {
            continue;
        }
        out.insert(name.clone(), value.clone());
    }

    // anthropic-version is required by the official API.
    if !out.contains_key("anthropic-version") {
        out.insert(
            HeaderName::from_static("anthropic-version"),
            HeaderValue::from_static(DEFAULT_ANTHROPIC_VERSION),
        );
    }

    if let Some(account) = account {
        inject_account_auth(&mut out, account)?;
    } else if let Some(auth) = fallback_auth.filter(|s| !s.is_empty()) {
        inject_fallback_auth(&mut out, auth)?;
    }

    Ok(out)
}

fn inject_account_auth(out: &mut HeaderMap, account: &AccountRef) -> Result<(), AppError> {
    if account.is_oauth {
        let hv = HeaderValue::from_str(&account.auth_header)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid oauth auth header: {e}")))?;
        out.insert(header::AUTHORIZATION, hv);
    } else {
        let key = strip_bearer(&account.auth_header);
        let hv = HeaderValue::from_str(key)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid x-api-key: {e}")))?;
        out.insert(HeaderName::from_static("x-api-key"), hv);
    }
    Ok(())
}

fn inject_fallback_auth(out: &mut HeaderMap, auth: &str) -> Result<(), AppError> {
    let auth = auth.trim();
    if auth.is_empty() {
        return Ok(());
    }
    if auth.to_ascii_lowercase().starts_with("bearer ") {
        let hv = HeaderValue::from_str(auth)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid upstream auth: {e}")))?;
        out.insert(header::AUTHORIZATION, hv);
    } else {
        let key = strip_bearer(auth);
        let hv = HeaderValue::from_str(key)
            .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid upstream x-api-key: {e}")))?;
        out.insert(HeaderName::from_static("x-api-key"), hv);
    }
    Ok(())
}

fn strip_bearer(s: &str) -> &str {
    s.strip_prefix("Bearer ")
        .or_else(|| s.strip_prefix("bearer "))
        .unwrap_or(s)
        .trim()
}

async fn send_upstream(
    client: &reqwest::Client,
    method: &Method,
    url: &str,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<reqwest::Response, AppError> {
    let mut builder = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::POST),
        url,
    );
    for (name, value) in headers.iter() {
        if let Ok(v) = value.to_str() {
            builder = builder.header(name.as_str(), v);
        }
    }
    if method != Method::GET && method != Method::HEAD {
        builder = builder.body(body);
    }
    builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("upstream request failed: {e}")))
}

fn copy_response_headers(upstream: &reqwest::Response) -> HeaderMap {
    let mut res_headers = HeaderMap::new();
    for (name, value) in upstream.headers().iter() {
        if is_hop_by_hop(name.as_str()) {
            continue;
        }
        if let Ok(n) = HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(v) = HeaderValue::from_bytes(value.as_bytes()) {
                res_headers.insert(n, v);
            }
        }
    }
    res_headers
}

fn stream_upstream_response(
    state: &AppState,
    member: &MemberContext,
    start: Instant,
    upstream_res: reqwest::Response,
    account: Option<&AccountRef>,
) -> Response {
    let status =
        StatusCode::from_u16(upstream_res.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let status_u16 = status.as_u16();
    let res_headers = copy_response_headers(&upstream_res);

    let mut metrics = StreamMetrics::new(member.key_id.clone(), status_u16, start);
    if let Some(a) = account {
        metrics = metrics.with_account(a);
    }
    let metrics = Arc::new(Mutex::new(Some(metrics)));
    let metrics_for_stream = Arc::clone(&metrics);
    let usage_guard = UsageOnComplete {
        metrics,
        usage_tx: state.usage_tx.clone(),
        live: state.live.clone(),
    };

    let byte_stream = upstream_res.bytes_stream();
    let client_stream = async_stream_map(byte_stream, metrics_for_stream, usage_guard);

    let mut response = Response::new(forward_io_stream(client_stream));
    *response.status_mut() = status;
    *response.headers_mut() = res_headers;
    response
}

/// Resolve accounts: anthropic pool → default pool → empty (caller uses TAGW_UPSTREAM).
fn resolve_accounts(state: &AppState) -> (Vec<AccountRef>, &'static str) {
    let anthropic = state.cache.enabled_accounts(ANTHROPIC_POOL_KEY);
    if !anthropic.is_empty() {
        return (anthropic, ANTHROPIC_POOL_KEY);
    }
    let default = state.cache.enabled_accounts(DEFAULT_POOL_KEY);
    (default, DEFAULT_POOL_KEY)
}

/// `POST /v1/messages` and `POST /v1/messages/count_tokens`.
pub async fn proxy_anthropic(
    State(state): State<AppState>,
    req: Request<Body>,
) -> Result<Response, AppError> {
    let member = authenticate(&state, req.headers())?;
    let start = Instant::now();

    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());

    let req_headers = req.headers().clone();
    let body_bytes = axum::body::to_bytes(req.into_body(), MAX_REQUEST_BODY)
        .await
        .map_err(|e| AppError::BadRequest(format!("request body: {e}")))?;

    let (pool_accounts, pool_key) = resolve_accounts(&state);

    if !pool_accounts.is_empty() {
        return proxy_with_router(
            &state,
            &member,
            start,
            &method,
            &path_and_query,
            &req_headers,
            body_bytes,
            &pool_accounts,
            pool_key,
        )
        .await;
    }

    // Dev fallback: TAGW_UPSTREAM when no pool accounts.
    let base = state
        .upstream_base
        .as_ref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::Upstream(
                "no anthropic/default accounts in pool and TAGW_UPSTREAM not configured".into(),
            )
        })?;
    let url = format!("{}{}", base.trim_end_matches('/'), path_and_query);
    let headers = build_upstream_headers(&req_headers, None, state.upstream_auth.as_deref())?;
    let upstream_res =
        send_upstream(&state.http_client, &method, &url, &headers, body_bytes).await?;
    Ok(stream_upstream_response(
        &state,
        &member,
        start,
        upstream_res,
        None,
    ))
}

async fn ensure_oauth_auth_header(
    state: &AppState,
    account: &mut AccountRef,
    force: bool,
) -> Result<(), AppError> {
    if !account.is_oauth {
        return Ok(());
    }
    let token = ensure_access_token_with_client(
        &state.db,
        &state.cache,
        &account.account_id,
        &state.http_client,
        force,
    )
    .await?;
    account.auth_header = format!("Bearer {token}");
    Ok(())
}

fn should_failover_status(status: u16, oauth_401_after_refresh: bool) -> bool {
    if AccountRouter::should_failover(status) {
        return true;
    }
    oauth_401_after_refresh && status == 401
}

async fn proxy_with_router(
    state: &AppState,
    member: &MemberContext,
    start: Instant,
    method: &Method,
    path_and_query: &str,
    req_headers: &HeaderMap,
    body_bytes: Bytes,
    accounts: &[AccountRef],
    pool_key: &str,
) -> Result<Response, AppError> {
    let mut last_error: Option<AppError> = None;
    let mut last_failover_response: Option<reqwest::Response> = None;
    let mut last_account: Option<AccountRef> = None;

    for attempt in 0..MAX_FAILOVER_ATTEMPTS {
        let Some(mut account) = state.account_router.pick(pool_key, accounts) else {
            break;
        };

        if let Err(e) = ensure_oauth_auth_header(state, &mut account, false).await {
            tracing::warn!(
                attempt,
                account_id = %account.account_id,
                error = %e,
                "oauth ensure_access_token failed before anthropic hop; trying next"
            );
            last_error = Some(e);
            last_account = Some(account);
            if attempt + 1 < MAX_FAILOVER_ATTEMPTS {
                continue;
            }
            break;
        }

        let url = format!(
            "{}{}",
            account.upstream_base.trim_end_matches('/'),
            path_and_query
        );
        let headers = match build_upstream_headers(req_headers, Some(&account), None) {
            Ok(h) => h,
            Err(e) => {
                last_error = Some(e);
                continue;
            }
        };

        tracing::debug!(
            attempt,
            account_id = %account.account_id,
            is_oauth = account.is_oauth,
            url = %url,
            "anthropic proxy attempt"
        );

        match send_upstream(
            &state.http_client,
            method,
            &url,
            &headers,
            body_bytes.clone(),
        )
        .await
        {
            Ok(mut upstream_res) => {
                let mut status_u16 = upstream_res.status().as_u16();
                let mut oauth_401_after_refresh = false;

                if status_u16 == 401 && account.is_oauth {
                    tracing::info!(
                        attempt,
                        account_id = %account.account_id,
                        "oauth anthropic 401; force-refresh and retry same account"
                    );
                    match ensure_oauth_auth_header(state, &mut account, true).await {
                        Ok(()) => {
                            let retry_headers =
                                match build_upstream_headers(req_headers, Some(&account), None) {
                                    Ok(h) => h,
                                    Err(e) => {
                                        last_error = Some(e);
                                        last_account = Some(account);
                                        if attempt + 1 < MAX_FAILOVER_ATTEMPTS {
                                            continue;
                                        }
                                        break;
                                    }
                                };
                            drop(upstream_res);
                            match send_upstream(
                                &state.http_client,
                                method,
                                &url,
                                &retry_headers,
                                body_bytes.clone(),
                            )
                            .await
                            {
                                Ok(retry_res) => {
                                    upstream_res = retry_res;
                                    status_u16 = upstream_res.status().as_u16();
                                    oauth_401_after_refresh = true;
                                }
                                Err(e) => {
                                    last_error = Some(e);
                                    last_account = Some(account);
                                    if attempt + 1 < MAX_FAILOVER_ATTEMPTS {
                                        continue;
                                    }
                                    break;
                                }
                            }
                        }
                        Err(e) => {
                            last_error = Some(e);
                            last_failover_response = Some(upstream_res);
                            last_account = Some(account);
                            if attempt + 1 < MAX_FAILOVER_ATTEMPTS {
                                continue;
                            }
                            break;
                        }
                    }
                }

                let can_retry = attempt + 1 < MAX_FAILOVER_ATTEMPTS
                    && should_failover_status(status_u16, oauth_401_after_refresh);
                if can_retry {
                    tracing::info!(
                        attempt,
                        status = status_u16,
                        account_id = %account.account_id,
                        "anthropic fail-over status before first byte; next account"
                    );
                    last_failover_response = Some(upstream_res);
                    last_account = Some(account);
                    continue;
                }
                return Ok(stream_upstream_response(
                    state,
                    member,
                    start,
                    upstream_res,
                    Some(&account),
                ));
            }
            Err(e) => {
                tracing::warn!(
                    attempt,
                    account_id = %account.account_id,
                    error = %e,
                    "anthropic upstream transport error"
                );
                last_error = Some(e);
                last_account = Some(account);
                if attempt + 1 < MAX_FAILOVER_ATTEMPTS {
                    continue;
                }
            }
        }
    }

    if let Some(res) = last_failover_response {
        return Ok(stream_upstream_response(
            state,
            member,
            start,
            res,
            last_account.as_ref(),
        ));
    }

    Err(last_error.unwrap_or_else(|| {
        AppError::Upstream("no upstream account available after fail-over attempts".into())
    }))
}

fn async_stream_map(
    byte_stream: impl futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    metrics: Arc<Mutex<Option<StreamMetrics>>>,
    usage_guard: UsageOnComplete,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    let mut stream = Box::pin(byte_stream);
    let mut guard = Some(usage_guard);
    futures_util::stream::poll_fn(move |cx| match stream.as_mut().poll_next(cx) {
        std::task::Poll::Ready(Some(Ok(chunk))) => {
            if let Ok(mut g) = metrics.lock() {
                if let Some(m) = g.as_mut() {
                    m.on_bytes(&chunk);
                }
            }
            std::task::Poll::Ready(Some(Ok(chunk)))
        }
        std::task::Poll::Ready(Some(Err(e))) => {
            if let Ok(mut g) = metrics.lock() {
                if let Some(m) = g.as_mut() {
                    m.error = Some(e.to_string());
                }
            }
            std::task::Poll::Ready(Some(Err(std::io::Error::other(e))))
        }
        std::task::Poll::Ready(None) => {
            drop(guard.take());
            std::task::Poll::Ready(None)
        }
        std::task::Poll::Pending => std::task::Poll::Pending,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_message_start_nested_usage() {
        let mut m = StreamMetrics::new("k".into(), 200, Instant::now());
        parse_usage_from_line(
            r#"data: {"type":"message_start","message":{"model":"claude-3","usage":{"input_tokens":12,"output_tokens":0}}}"#,
            &mut m,
        );
        assert!(!m.usage_incomplete);
        assert_eq!(m.prompt_tokens, 12);
        assert_eq!(m.completion_tokens, 0);
        assert_eq!(m.model.as_deref(), Some("claude-3"));
    }

    #[test]
    fn parse_message_delta_output_tokens() {
        let mut m = StreamMetrics::new("k".into(), 200, Instant::now());
        m.prompt_tokens = 12;
        m.usage_incomplete = false;
        parse_usage_from_line(
            r#"data: {"type":"message_delta","usage":{"output_tokens":4}}"#,
            &mut m,
        );
        assert_eq!(m.completion_tokens, 4);
        assert_eq!(m.prompt_tokens, 12);
    }

    #[test]
    fn strip_bearer_helpers() {
        assert_eq!(strip_bearer("Bearer sk-abc"), "sk-abc");
        assert_eq!(strip_bearer("sk-abc"), "sk-abc");
    }
}
