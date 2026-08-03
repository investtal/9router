//! OpenAI-compatible `/v1/*` reverse proxy with streaming passthrough.
//!
//! Account selection via [`AccountRouter`] (round-robin + fail-over). When the
//! account pool is empty, falls back to `TAGW_UPSTREAM` (dev).

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
use crate::state::{AppState, DEFAULT_POOL_KEY};
use crate::usage::{estimate_cost, UsageEvent};

/// Max side-channel line buffer for usage parse. Cap prevents unbounded hold of
/// body-ish data when upstream never sends a newline (client stream is unaffected).
const LINE_BUF_MAX: usize = 256 * 1024;

/// Max request body collected for fail-over retries (LLM JSON; not the response stream).
const MAX_REQUEST_BODY: usize = 32 * 1024 * 1024;

/// Hop-by-hop headers that must not be forwarded (RFC 7230 §6.1).
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
            | "content-length" // recomputed by reqwest / hyper for streamed bodies
    )
}

/// Extract `Bearer <token>` from the Authorization header.
fn extract_bearer(headers: &HeaderMap) -> Option<&str> {
    let val = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = val.strip_prefix("Bearer ").or_else(|| val.strip_prefix("bearer "))?;
    let token = token.trim();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

/// Authenticate member from request headers via ConfigCache.
fn authenticate(state: &AppState, headers: &HeaderMap) -> Result<MemberContext, AppError> {
    let token = extract_bearer(headers).ok_or(AppError::Unauthorized)?;
    state
        .cache
        .authenticate_bearer(token)
        .ok_or(AppError::Unauthorized)
}

/// Best-effort parse of OpenAI-style `usage` objects from SSE `data:` lines or JSON bodies.
fn parse_usage_from_line(line: &str, metrics: &mut StreamMetrics) {
    let line = line.trim();
    if line.is_empty() || line == "data: [DONE]" {
        return;
    }
    let json_str = line.strip_prefix("data:").map(str::trim).unwrap_or(line);
    if json_str.is_empty() || json_str == "[DONE]" {
        return;
    }
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json_str) else {
        return;
    };
    if let Some(model) = v.get("model").and_then(|m| m.as_str()) {
        if metrics.model.is_none() {
            metrics.model = Some(model.to_string());
        }
    }
    let Some(usage) = v.get("usage") else {
        return;
    };
    if usage.is_null() {
        return;
    }
    let prompt = usage
        .get("prompt_tokens")
        .or_else(|| usage.get("input_tokens"))
        .and_then(|x| x.as_i64());
    let completion = usage
        .get("completion_tokens")
        .or_else(|| usage.get("output_tokens"))
        .and_then(|x| x.as_i64());
    let cached = usage
        .get("cached_tokens")
        .or_else(|| {
            usage
                .get("prompt_tokens_details")
                .and_then(|d| d.get("cached_tokens"))
        })
        .and_then(|x| x.as_i64())
        .unwrap_or(0);

    if let Some(p) = prompt {
        metrics.prompt_tokens = p;
    }
    if let Some(c) = completion {
        metrics.completion_tokens = c;
    }
    metrics.cached_tokens = cached;
    // Any usage object with token fields counts as complete enough.
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
        // Side-channel line scan only — never holds the body for the client.
        if let Ok(s) = std::str::from_utf8(chunk) {
            self.line_buf.push_str(s);
            self.drain_complete_lines();
            // Cap: oversized buffer without newline must not retain full-body-ish data.
            if self.line_buf.len() > LINE_BUF_MAX {
                tracing::warn!(
                    len = self.line_buf.len(),
                    cap = LINE_BUF_MAX,
                    "usage line_buf exceeded cap without newline; clearing (usage incomplete)"
                );
                self.line_buf.clear();
                self.usage_incomplete = true;
            }
        }
    }

    /// Drain newline-terminated lines from `line_buf` into the usage parser.
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

    /// At end-of-stream, parse any remaining buffer (non-newline-terminated JSON bodies).
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
        // EOS: flush residual line_buf so non-stream JSON without trailing `\n` still parses.
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
            tool: Some("openai".into()),
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

/// Drop guard: enqueue usage when the response stream finishes or the client disconnects.
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
                // Non-blocking: never await the writer; log if the channel is full/closed.
                if let Err(e) = self.usage_tx.try_send(ev) {
                    tracing::warn!(
                        error = %e,
                        "usage channel full or closed; dropping UsageEvent"
                    );
                }
            }
        }
    }
}

/// Build hop-by-hop-stripped headers for upstream; set Authorization from target.
fn build_upstream_headers(
    req_headers: &HeaderMap,
    auth: Option<&str>,
) -> Result<HeaderMap, AppError> {
    let mut out_headers = HeaderMap::new();
    for (name, value) in req_headers.iter() {
        if is_hop_by_hop(name.as_str()) {
            continue;
        }
        // Client bearer authenticates to us; do not forward member key upstream.
        if name == header::AUTHORIZATION {
            continue;
        }
        out_headers.insert(name.clone(), value.clone());
    }
    if let Some(auth) = auth {
        if !auth.is_empty() {
            let hv = HeaderValue::from_str(auth)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid upstream auth: {e}")))?;
            out_headers.insert(header::AUTHORIZATION, hv);
        }
    }
    Ok(out_headers)
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

/// Build a streaming client response from a successful upstream response.
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

    // CRITICAL: use bytes_stream() — never response.bytes().await on the stream path.
    // After first byte is forwarded there is no account switch (fail-over only pre-body).
    let byte_stream = upstream_res.bytes_stream();
    let client_stream = async_stream_map(byte_stream, metrics_for_stream, usage_guard);

    let mut response = Response::new(forward_io_stream(client_stream));
    *response.status_mut() = status;
    *response.headers_mut() = res_headers;
    response
}

/// Catch-all OpenAI-compatible proxy for `/v1/*`.
pub async fn proxy_openai(
    State(state): State<AppState>,
    req: Request<Body>,
) -> Result<Response, AppError> {
    let member = authenticate(&state, req.headers())?;
    // latency_ms / ttft_ms are measured from after auth (proxy hop + upstream).
    let start = Instant::now();

    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());

    // Collect request body once so fail-over can re-send (response path still streams).
    let req_headers = req.headers().clone();
    let body_bytes = axum::body::to_bytes(req.into_body(), MAX_REQUEST_BODY)
        .await
        .map_err(|e| AppError::BadRequest(format!("request body: {e}")))?;

    let pool_accounts = state.cache.enabled_accounts(DEFAULT_POOL_KEY);

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
        )
        .await;
    }

    // Dev fallback: pure TAGW_UPSTREAM when pool is empty.
    let base = state
        .upstream_base
        .as_ref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::Upstream(
                "no accounts in pool and TAGW_UPSTREAM not configured".into(),
            )
        })?;
    let url = format!("{}{}", base.trim_end_matches('/'), path_and_query);
    let headers = build_upstream_headers(&req_headers, state.upstream_auth.as_deref())?;
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

/// Ensure a fresh Bearer for an OAuth account (skewed refresh when near expiry).
/// Updates `account.auth_header` in place. No-op for API-key accounts.
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

/// Whether this pre-body status should fail over to another account.
///
/// OAuth 401 is only fail-over-eligible **after** a same-account force-refresh
/// retry has already been attempted (see [`proxy_with_router`]).
fn should_failover_status(status: u16, oauth_401_after_refresh: bool) -> bool {
    if AccountRouter::should_failover(status) {
        return true;
    }
    oauth_401_after_refresh && status == 401
}

/// Round-robin pick + fail-over loop (status check before any client body byte).
///
/// OAuth accounts:
/// 1. `ensure_access_token` before each upstream hop (refresh if near expiry).
/// 2. On upstream 401: force-refresh once, retry **same** account once, then fail-over.
/// Never switches accounts after the first client body byte.
async fn proxy_with_router(
    state: &AppState,
    member: &MemberContext,
    start: Instant,
    method: &Method,
    path_and_query: &str,
    req_headers: &HeaderMap,
    body_bytes: Bytes,
    accounts: &[AccountRef],
) -> Result<Response, AppError> {
    let mut last_error: Option<AppError> = None;
    let mut last_failover_response: Option<reqwest::Response> = None;
    let mut last_account: Option<AccountRef> = None;

    for attempt in 0..MAX_FAILOVER_ATTEMPTS {
        let Some(mut account) = state.account_router.pick(DEFAULT_POOL_KEY, accounts) else {
            break;
        };

        // Pre-hop OAuth ensure (near-expiry / missing freshness in pool cache).
        if let Err(e) = ensure_oauth_auth_header(state, &mut account, false).await {
            tracing::warn!(
                attempt,
                account_id = %account.account_id,
                error = %e,
                "oauth ensure_access_token failed before hop; trying next account"
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
        let headers = match build_upstream_headers(req_headers, Some(&account.auth_header)) {
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
            "proxy attempt"
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

                // OAuth 401: force-refresh once + retry same account once (still pre-body).
                if status_u16 == 401 && account.is_oauth {
                    tracing::info!(
                        attempt,
                        account_id = %account.account_id,
                        "oauth upstream 401; force-refresh and retry same account"
                    );
                    match ensure_oauth_auth_header(state, &mut account, true).await {
                        Ok(()) => {
                            let retry_headers = match build_upstream_headers(
                                req_headers,
                                Some(&account.auth_header),
                            ) {
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
                            // Drop first 401 without forwarding any bytes.
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
                                    tracing::warn!(
                                        attempt,
                                        account_id = %account.account_id,
                                        error = %e,
                                        "oauth same-account retry transport error"
                                    );
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
                            tracing::warn!(
                                attempt,
                                account_id = %account.account_id,
                                error = %e,
                                "oauth force-refresh after 401 failed; fail-over"
                            );
                            last_error = Some(e);
                            // Keep original 401 for final stream if no more accounts.
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
                        "upstream fail-over status before first byte; trying next account"
                    );
                    // Drop response without forwarding any bytes to the client.
                    last_failover_response = Some(upstream_res);
                    last_account = Some(account);
                    continue;
                }
                // Forward: either success, non-failover error, or attempts exhausted.
                // After this point first client body byte commits us to this upstream.
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
                    "upstream transport error"
                );
                last_error = Some(e);
                last_account = Some(account);
                // Transport failure: no response bytes to client → allow fail-over.
                if attempt + 1 < MAX_FAILOVER_ATTEMPTS {
                    continue;
                }
            }
        }
    }

    // Exhausted attempts: if we have a last fail-over response, stream it so the
    // client sees the real upstream status (e.g. final 429).
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

/// Map reqwest byte stream → io errors, update metrics side-channel, hold usage guard.
fn async_stream_map(
    byte_stream: impl futures_util::Stream<Item = Result<Bytes, reqwest::Error>> + Send + 'static,
    metrics: Arc<Mutex<Option<StreamMetrics>>>,
    usage_guard: UsageOnComplete,
) -> impl futures_util::Stream<Item = Result<Bytes, std::io::Error>> + Send + 'static {
    // Pin usage_guard inside the stream so Drop runs when the body is fully consumed
    // or dropped (client disconnect).
    let mut stream = Box::pin(byte_stream);
    let mut guard = Some(usage_guard);
    futures_util::stream::poll_fn(move |cx| {
        match stream.as_mut().poll_next(cx) {
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
                // Stream complete — drop guard to enqueue UsageEvent.
                drop(guard.take());
                std::task::Poll::Ready(None)
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn flush_line_buf_parses_non_newline_terminated_json() {
        let mut m = StreamMetrics::new("key-1".into(), 200, Instant::now());
        // Simulate a full non-stream JSON body delivered in one chunk with no trailing `\n`.
        let body = r#"{"id":"cmpl","model":"gpt-4o","usage":{"prompt_tokens":11,"completion_tokens":3,"prompt_tokens_details":{"cached_tokens":2}}}"#;
        m.on_bytes(&Bytes::from(body));
        // Still buffered (no newline), usage not complete yet.
        assert!(m.usage_incomplete);
        assert_eq!(m.prompt_tokens, 0);

        m.flush_line_buf();
        assert!(!m.usage_incomplete);
        assert_eq!(m.prompt_tokens, 11);
        assert_eq!(m.completion_tokens, 3);
        assert_eq!(m.cached_tokens, 2);
        assert_eq!(m.model.as_deref(), Some("gpt-4o"));
        assert!(m.line_buf.is_empty());
    }

    #[test]
    fn into_event_flushes_residual_line_buf() {
        let mut m = StreamMetrics::new("key-1".into(), 200, Instant::now());
        m.on_bytes(&Bytes::from(
            r#"{"model":"gpt-4o","usage":{"prompt_tokens":5,"completion_tokens":1}}"#,
        ));
        let ev = m.into_event();
        assert!(!ev.usage_incomplete);
        assert_eq!(ev.prompt_tokens, 5);
        assert_eq!(ev.completion_tokens, 1);
        assert!(ev.latency_ms.is_some());
        assert_eq!(ev.model.as_deref(), Some("gpt-4o"));
        assert_eq!(ev.member_key_id.as_deref(), Some("key-1"));
        assert_eq!(ev.status, Some(200));
    }

    #[test]
    fn line_buf_cap_clears_oversized_buffer_without_newline() {
        let mut m = StreamMetrics::new("key-1".into(), 200, Instant::now());
        // One huge chunk with no newline exceeds LINE_BUF_MAX.
        let huge = "x".repeat(LINE_BUF_MAX + 1);
        m.on_bytes(&Bytes::from(huge));
        assert!(m.line_buf.is_empty(), "oversized line_buf must be cleared");
        assert!(m.usage_incomplete);

        // Subsequent newline-terminated usage can still parse.
        m.on_bytes(&Bytes::from(
            "data: {\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":1}}\n",
        ));
        assert!(!m.usage_incomplete);
        assert_eq!(m.prompt_tokens, 9);
    }

    #[test]
    fn newline_terminated_sse_usage_parses_without_explicit_flush() {
        let mut m = StreamMetrics::new("key-1".into(), 200, Instant::now());
        m.on_bytes(&Bytes::from(
            "data: {\"id\":\"c\",\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n",
        ));
        assert!(!m.usage_incomplete);
        assert_eq!(m.prompt_tokens, 10);
        assert_eq!(m.completion_tokens, 2);
        assert!(m.line_buf.is_empty());
    }
}
