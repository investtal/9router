//! OpenAI-compatible `/v1/*` reverse proxy with streaming passthrough.
//!
//! Temporary single upstream from `AppState.upstream_*` (Task 5). AccountRouter is Task 6.

use std::sync::{Arc, Mutex};
use std::time::Instant;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderName, HeaderValue, Method, Request, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use futures_util::StreamExt;

use crate::auth::member_key::MemberContext;
use crate::error::AppError;
use crate::proxy::stream::forward_io_stream;
use crate::state::AppState;
use crate::usage::{estimate_cost, UsageEvent};

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
        }
    }

    fn on_bytes(&mut self, chunk: &Bytes) {
        if self.first_byte_at.is_none() && !chunk.is_empty() {
            self.first_byte_at = Some(Instant::now());
        }
        // Side-channel line scan only — never holds the body for the client.
        if let Ok(s) = std::str::from_utf8(chunk) {
            self.line_buf.push_str(s);
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
    }

    fn into_event(self) -> UsageEvent {
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
            provider_id: None,
            account_id: None,
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
}

impl Drop for UsageOnComplete {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.metrics.lock() {
            if let Some(m) = guard.take() {
                let _ = self.usage_tx.try_send(m.into_event());
            }
        }
    }
}

/// Catch-all OpenAI-compatible proxy for `/v1/*`.
pub async fn proxy_openai(
    State(state): State<AppState>,
    req: Request<Body>,
) -> Result<Response, AppError> {
    let member = authenticate(&state, req.headers())?;
    // latency_ms / ttft_ms are measured from after auth (proxy hop + upstream).
    let start = Instant::now();

    let upstream_base = state
        .upstream_base
        .as_ref()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| AppError::Upstream("TAGW_UPSTREAM not configured".into()))?;

    let method = req.method().clone();
    let path_and_query = req
        .uri()
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or_else(|| req.uri().path());

    let base = upstream_base.trim_end_matches('/');
    let url = format!("{base}{path_and_query}");

    // Build upstream request headers: forward most, strip hop-by-hop, set upstream auth.
    let mut out_headers = HeaderMap::new();
    for (name, value) in req.headers().iter() {
        if is_hop_by_hop(name.as_str()) {
            continue;
        }
        // Client bearer authenticates to us; do not forward member key upstream.
        if name == header::AUTHORIZATION {
            continue;
        }
        out_headers.insert(name.clone(), value.clone());
    }
    if let Some(auth) = state.upstream_auth.as_ref() {
        if !auth.is_empty() {
            let hv = HeaderValue::from_str(auth)
                .map_err(|e| AppError::Internal(anyhow::anyhow!("invalid upstream auth: {e}")))?;
            out_headers.insert(header::AUTHORIZATION, hv);
        }
    }

    // Stream request body to upstream (no full-body collect).
    let req_body = req.into_body();
    let body_stream = req_body.into_data_stream().map(|r| {
        r.map_err(|e| std::io::Error::other(format!("request body stream error: {e}")))
    });
    let upstream_body = reqwest::Body::wrap_stream(body_stream);

    let client = state.http_client.clone();
    let mut builder = client.request(
        reqwest::Method::from_bytes(method.as_str().as_bytes()).unwrap_or(reqwest::Method::POST),
        &url,
    );
    for (name, value) in out_headers.iter() {
        if let Ok(v) = value.to_str() {
            builder = builder.header(name.as_str(), v);
        }
    }
    // Empty GET/HEAD bodies are fine with wrap_stream of empty stream.
    if method != Method::GET && method != Method::HEAD {
        builder = builder.body(upstream_body);
    }

    let upstream_res = builder
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("upstream request failed: {e}")))?;

    let status =
        StatusCode::from_u16(upstream_res.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let status_u16 = status.as_u16();

    // Copy response headers (strip hop-by-hop).
    let mut res_headers = HeaderMap::new();
    for (name, value) in upstream_res.headers().iter() {
        if is_hop_by_hop(name.as_str()) {
            continue;
        }
        if let Ok(n) = HeaderName::from_bytes(name.as_str().as_bytes()) {
            if let Ok(v) = HeaderValue::from_bytes(value.as_bytes()) {
                res_headers.insert(n, v);
            }
        }
    }

    let metrics = Arc::new(Mutex::new(Some(StreamMetrics::new(
        member.key_id.clone(),
        status_u16,
        start,
    ))));
    let metrics_for_stream = Arc::clone(&metrics);
    let _usage_guard = UsageOnComplete {
        metrics,
        usage_tx: state.usage_tx.clone(),
    };
    // Keep guard alive for the lifetime of the body stream by moving into the stream.
    let usage_guard = _usage_guard;

    // CRITICAL: use bytes_stream() — never response.bytes().await on the stream path.
    let byte_stream = upstream_res.bytes_stream();
    let client_stream = async_stream_map(byte_stream, metrics_for_stream, usage_guard);

    let mut response = Response::new(forward_io_stream(client_stream));
    *response.status_mut() = status;
    *response.headers_mut() = res_headers;
    Ok(response)
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


