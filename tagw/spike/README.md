# HTTP stack spike (axum vs Pingora)

## v1 decision

**Stay on axum/hyper.** See ADR:

[`docs/adr/2026-08-03-proxy-http-stack.md`](../../docs/adr/2026-08-03-proxy-http-stack.md)

Evidence: Task 18 SLO smoke on axum (sequential ~13ms client-added TTFB including argon2; 50 concurrent streams stable). Pingora not required for v1.

## Reproduce axum smoke (baseline)

From repo root (or `tagw/`):

```bash
# Prefer release binary for lower variance
cd tagw && cargo build --release -p tagw

./scripts/slo_smoke.sh
# or from repo root:
# ./tagw/scripts/slo_smoke.sh
```

Useful env vars:

| Variable | Default | Meaning |
|----------|---------|---------|
| `CONCURRENCY` | `50` | Parallel streams for stability leg |
| `SEQ_SAMPLES` | `30` | Sequential samples for hop-added TTFB |
| `SLO_ADDED_P95_MS` | `250` | Auth-inclusive sequential added p95 budget |
| `SLO_MAX_TTFB_MS` | `5000` | Concurrent max TTFB stall guard |
| `TAGW_BIN` | `target/release/tagw` | Override binary path |

CI note: shared runners are noisy; treat single failures as flaky unless repeated on a quiet machine.

## When to run a Pingora prototype

Only if [ADR revisit criteria](../../docs/adr/2026-08-03-proxy-http-stack.md#revisit-criteria-switch-to-pingora-only-if) fire. Suggested minimal spike:

1. Create `tagw/spike/pingora_proxy/` crate depending on `pingora` / `pingora-proxy`.
2. Implement a single filter: forward `POST /v1/chat/completions` to the same `mock_upstream.py` with streaming body.
3. **Do not** port admin/OAuth in the spike — measure proxy hop only.
4. Point a copy of `slo_smoke.sh` at the Pingora binary (skip member-key auth or use a no-op auth layer so comparison is fair against axum **post-auth** hop).
5. Record TTFB p50/p95/p99, RSS, and CPU @ 50 streams into the ADR table.
6. Decide: switch only on clear p95/p99 win or simpler multi-upstream ops without dual-stack admin pain.

## Optional: axum post-auth hop only

Internal `ttft_ms` on usage events is measured after member auth. After a smoke run you can inspect SQLite:

```bash
sqlite3 "$TAGW_DATA_DIR/gateway.db" \
  "SELECT avg(ttft_ms), max(ttft_ms) FROM usage_events WHERE ttft_ms IS NOT NULL;"
```

(Schema/column names may vary slightly — check `schema.sql`.)

This isolates proxy+upstream TTFB from argon2 verify cost.
