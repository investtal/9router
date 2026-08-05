> **Moved to** [`investtal/agi`](https://github.com/investtal/agi) (AI Gateway Investtal). This copy is retained for history; edit the agi repo.

# ADR: Proxy HTTP stack — axum/hyper vs Pingora

- **Status:** Accepted
- **Date:** 2026-08-03
- **Deciders:** Team AI Gateway (tagw)
- **Related:** `docs/specs/2026-08-03-team-ai-gateway-design.md` §13; Task 18 SLO smoke

## Context

tagw is a **single Rust binary** that serves:

1. OpenAI-compatible + Anthropic **streaming reverse proxy** (`/v1/*`)
2. Admin/control REST + OAuth callbacks (`/api/*`)
3. Live log SSE + static TanStack SPA

The design defaulted to **axum/hyper/tokio**, with Pingora spike-gated: switch only if concurrent SSE + multi-upstream filters clearly win p95/p99 **or** simplify fail-over without admin UX regressions.

Latency SLOs (design §4):

| Metric | Target |
|--------|--------|
| Gateway hop p95 (non-stream, mock) | &lt; 20ms added |
| Stream TTFB p95 (mock) | &lt; 10ms added vs direct mock |
| TTFT p99 added | &lt; 50ms |
| Stream buffering | Zero full-body buffer of request/response streams |
| Concurrent streams | 15+ (smoke uses 50) stable |

## Options

### A. Stay on axum/hyper (current)

- One process, one router: proxy + admin + OAuth + SPA share `AppState`, extractors, middleware.
- Streaming path already uses `reqwest` `bytes_stream()` → client body without full buffer (Task 5/9 tests).
- AccountRouter RR + pre-body fail-over implemented in-process.
- Team familiarity high; crate ecosystem (axum 0.8, tower-http, reqwest) already wired.

### B. Switch hot path to Pingora

- Proven L7 proxy filters, connection pooling, and multi-upstream patterns at Cloudflare scale.
- Separate programming model (filters/sessions) from axum admin surface → **two HTTP stacks** or full rewrite of admin onto Pingora (awkward).
- Higher learning cost; dual-runtime complexity for a single-VPS team gateway.

## Measurements (Task 18 smoke)

Harness: `tagw/scripts/slo_smoke.sh` + `tagw/scripts/mock_upstream.py`  
Machine: local quiet dev (Apple Silicon); release `tagw`; mock SSE first-byte immediate; member-key argon2 on every gateway request.

**Sequential hop estimate** (workers=1, n=30) — client TTFB includes argon2 verify:

| Path | p50 (ms) | p95 (ms) | p99 (ms) |
|------|----------|----------|----------|
| Direct mock | 0.9 | 1.7 | 10.0 |
| Through tagw (axum) | 13.5 | 14.7 | 22.6 |
| **Added (gw − mock)** | **12.6** | **13.0** | **12.5** |

**Concurrent stability** (50 parallel streams through tagw):

| Metric | Value |
|--------|-------|
| All streams succeed | yes |
| Gateway TTFB p50 / p95 / p99 | ~129 / ~142 / ~144 ms |
| Gateway max TTFB | ~144 ms (≪ multi-second stall) |

Notes:

- Client-measured “added” includes **argon2 member-key verify** on every request. Internal usage metrics start **after** auth (`proxy_openai` `Instant::now()` post-authenticate), so post-auth hop is a subset of the ~13ms sequential add.
- Design stream target (&lt;10ms added) is **post-auth hop**. Sequential client add ~13ms with auth is consistent with that target once argon2 is excluded; smoke default assertion budget is 250ms (auth-inclusive + CI noise) with stall guard 5s.
- Pingora was **not** reimplemented for a side-by-side binary in this ADR window. Decision uses axum smoke evidence + product cohesion (see default in plan Task 19).

### Comparison table (decision record)

| Metric | axum/hyper (measured / assessed) | Pingora (not measured this cycle) |
|--------|----------------------------------|-----------------------------------|
| TTFB p95 (seq, client, auth incl.) | **~15 ms** absolute; **~13 ms added** | Not measured |
| TTFB p99 (seq, client, auth incl.) | **~23 ms** absolute; **~13 ms added** | Not measured |
| CPU @ 50 streams | Acceptable (no stalls; max TTFB ~144 ms) | Not measured |
| Admin REST + OAuth + SPA cohesion | **Natural** (single axum app) | Awkward / dual stack |
| Streaming reverse proxy | Excellent (DIY pipe; tests prove no full buffer) | Excellent (filter model) |
| Team familiarity | Higher | Lower |
| Decision | **Stay axum** | Revisit if criteria below met |

## Decision

**Stay on axum/hyper for v1.**

Justification (plan default, confirmed by smoke):

> Admin + proxy cohesion; SLOs addressed via stream design (zero full-body buffer, pre-body fail-over only) + concurrent stream smoke; sequential client hop ~13ms including auth on quiet hardware.

## Consequences

### Positive

- One HTTP stack, one binary, one mental model for ops and contributors.
- Existing tests (`proxy_stream`, `anthropic_stream`, `router_rr`) remain authoritative for protocol correctness.
- No rewrite risk before product sign-off (Task 20 checklist).

### Negative / accepted risks

- Absolute client TTFB dominated by **argon2 verify** until a verified-token cache (or cheaper KDF) is added; design hop SLO should be reported post-auth or after such a cache.
- Pingora’s multi-upstream filter ergonomics not exploited; fail-over remains hand-rolled (already implemented and tested).

## Revisit criteria (switch to Pingora only if)

Re-open this ADR and spike a Pingora filter path when **any** of:

1. **Latency:** Under production-like load, axum stream TTFB p95/p99 **post-auth** exceeds design targets by a clear margin **and** a Pingora prototype on the same mock workload wins by ≥20% p95 **or** halves p99 tails.
2. **Concurrency:** ≥50 parallel long-lived SSE streams show multi-second stalls or unbounded memory attributable to the axum/hyper path (not SQLite/usage writer).
3. **Fail-over complexity:** Multi-pool / health-checked upstream management becomes significantly simpler as Pingora filters than as in-process `AccountRouter` without regressing admin/OAuth UX.
4. **Ops requirement:** Need connection-level features (e.g. advanced load shedding, kernel-bypass adjacent) that axum cannot meet without equivalent complexity.

Spike procedure (when triggered): see `tagw/spike/README.md`.

## References

- Design §13 Pingora evaluation plan
- `tagw/scripts/slo_smoke.sh` — how to reproduce numbers
- Integration tests: `tagw/crates/tagw/tests/proxy_stream.rs`, `router_rr.rs`
