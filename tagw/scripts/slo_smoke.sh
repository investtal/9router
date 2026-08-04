#!/usr/bin/env bash
# SLO smoke: concurrent streaming TTFB through tagw vs direct mock.
#
# Design target (post-auth hop): p95 added TTFB < 10ms stream (see design §4).
# Client-side measurement includes member-key argon2 verify on every request, so
# the default assertion uses SLO_ADDED_P95_MS (default 250) to catch multi-second
# stalls while remaining green on a quiet local machine. Set SLO_ADDED_P95_MS=10
# only when comparing stacks without auth cost, or after a verified-token cache.
#
# Usage (from repo root or tagw/):
#   ./tagw/scripts/slo_smoke.sh
#
# Exit 0 on pass, non-zero on failure.
#
# CI note: shared runners are noisy; treat failures as flaky unless repeated.
# Prefer local quiet machine for gating.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
TAGW_DIR="${ROOT}/tagw"
SCRIPTS="${TAGW_DIR}/scripts"
MOCK_PY="${SCRIPTS}/mock_upstream.py"
BIN="${TAGW_BIN:-${TAGW_DIR}/target/release/tagw}"
DEBUG_BIN="${TAGW_DIR}/target/debug/tagw"

CONCURRENCY="${CONCURRENCY:-50}"
# Sequential samples for hop-added TTFB (avoids mock thread-pool skew under load).
SEQ_SAMPLES="${SEQ_SAMPLES:-30}"
# Client-measured sequential added p95 budget (ms). Includes argon2 member-key verify.
# Design hop target is 10ms *post-auth*; default budget is auth-inclusive.
SLO_ADDED_P95_MS="${SLO_ADDED_P95_MS:-250}"
# Absolute stall budget (ms) for any single concurrent gateway request.
SLO_MAX_TTFB_MS="${SLO_MAX_TTFB_MS:-5000}"
WARMUP="${WARMUP:-3}"
ADMIN_USER="${TAGW_ADMIN_USER:-admin}"
ADMIN_PASS="${TAGW_ADMIN_PASSWORD:-admin}"

# Prefer workspace tmp so sandbox/CI restrictions on /var/folders do not bite.
WORKDIR="${TAGW_SLO_WORKDIR:-${TAGW_DIR}/.slo-smoke-tmp}"
rm -rf "${WORKDIR}"
mkdir -p "${WORKDIR}"
cleanup() {
  set +e
  if [[ -n "${TAGW_PID:-}" ]]; then kill "${TAGW_PID}" 2>/dev/null; wait "${TAGW_PID}" 2>/dev/null; fi
  if [[ -n "${MOCK_PID:-}" ]]; then kill "${MOCK_PID}" 2>/dev/null; wait "${MOCK_PID}" 2>/dev/null; fi
  rm -rf "${WORKDIR}"
}
trap cleanup EXIT

log() { printf '[slo_smoke] %s\n' "$*"; }
die() { log "FAIL: $*"; exit 1; }

need() { command -v "$1" >/dev/null 2>&1 || die "missing dependency: $1"; }
need python3
need curl
need jq

# Prefer release binary for realistic latency; fall back to debug build.
ensure_bin() {
  if [[ -x "${BIN}" ]]; then
    return 0
  fi
  if [[ -x "${DEBUG_BIN}" ]]; then
    BIN="${DEBUG_BIN}"
    log "using debug binary (build --release for lower variance): ${BIN}"
    return 0
  fi
  log "building release tagw..."
  (cd "${TAGW_DIR}" && cargo build --release -p tagw)
  BIN="${TAGW_DIR}/target/release/tagw"
  [[ -x "${BIN}" ]] || die "binary not found after build"
}

free_port() {
  python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
}

start_mock() {
  local port_file="${WORKDIR}/mock.port"
  rm -f "${port_file}"
  # -u: unbuffered so LISTENING is visible immediately under redirection.
  python3 -u "${MOCK_PY}" --host 127.0.0.1 --port 0 --chunk-delay-ms 5 \
    --port-file "${port_file}" \
    >"${WORKDIR}/mock.out" 2>"${WORKDIR}/mock.err" &
  MOCK_PID=$!
  for _ in $(seq 1 100); do
    if [[ -s "${port_file}" ]]; then
      MOCK_ADDR="$(tr -d '[:space:]' < "${port_file}")"
      MOCK_BASE="http://${MOCK_ADDR}"
      if curl -sf --max-time 1 "${MOCK_BASE}/healthz" >/dev/null 2>&1; then
        log "mock upstream at ${MOCK_BASE}"
        return 0
      fi
    fi
    if ! kill -0 "${MOCK_PID}" 2>/dev/null; then
      cat "${WORKDIR}/mock.err" >&2 || true
      cat "${WORKDIR}/mock.out" >&2 || true
      die "mock upstream exited before ready"
    fi
    sleep 0.05
  done
  cat "${WORKDIR}/mock.err" >&2 || true
  cat "${WORKDIR}/mock.out" >&2 || true
  die "mock upstream did not start"
}

start_tagw() {
  local bind_port data_dir
  bind_port="$(free_port)"
  data_dir="${WORKDIR}/data"
  mkdir -p "${data_dir}"
  TAGW_BIND="127.0.0.1:${bind_port}" \
  TAGW_DATA_DIR="${data_dir}" \
  TAGW_UPSTREAM="${MOCK_BASE}" \
  TAGW_UPSTREAM_AUTH="Bearer mock-upstream" \
  TAGW_ADMIN_PASSWORD="${ADMIN_PASS}" \
  TAGW_SESSION_SECRET="slo-smoke-session-secret" \
  RUST_LOG="${RUST_LOG:-warn}" \
    "${BIN}" >"${WORKDIR}/tagw.out" 2>"${WORKDIR}/tagw.err" &
  TAGW_PID=$!
  GW_BASE="http://127.0.0.1:${bind_port}"
  for _ in $(seq 1 100); do
    if curl -sf "${GW_BASE}/healthz" >/dev/null 2>&1; then
      log "tagw listening at ${GW_BASE}"
      return 0
    fi
    # binary may have exited
    if ! kill -0 "${TAGW_PID}" 2>/dev/null; then
      cat "${WORKDIR}/tagw.err" >&2 || true
      die "tagw exited before healthz"
    fi
    sleep 0.05
  done
  cat "${WORKDIR}/tagw.err" >&2 || true
  die "tagw healthz timeout"
}

create_member_key() {
  local jar key_json
  jar="${WORKDIR}/cookies.txt"
  curl -sf -c "${jar}" -b "${jar}" \
    -X POST "${GW_BASE}/api/auth/login" \
    -H 'content-type: application/json' \
    -d "{\"username\":\"${ADMIN_USER}\",\"password\":\"${ADMIN_PASS}\"}" \
    >/dev/null \
    || die "admin login failed (user=${ADMIN_USER})"
  key_json="$(curl -sf -b "${jar}" \
    -X POST "${GW_BASE}/api/admin/keys" \
    -H 'content-type: application/json' \
    -d '{"name":"slo-smoke"}')" \
    || die "create member key failed"
  MEMBER_KEY="$(printf '%s' "${key_json}" | jq -r '.key')"
  [[ -n "${MEMBER_KEY}" && "${MEMBER_KEY}" != "null" ]] || die "no key in create response: ${key_json}"
  log "member key created (prefix $(printf '%s' "${MEMBER_KEY}" | cut -c1-8)...)"
}

# Measure TTFB for N POSTs. concurrent=1 → sequential; concurrent=N → N parallel.
measure_ttfb_batch() {
  local base_url auth_header n out_file workers
  base_url="$1"
  auth_header="$2" # may be empty
  n="$3"
  out_file="$4"
  workers="${5:-$n}" # max parallel workers

  python3 - "${base_url}" "${auth_header}" "${n}" "${out_file}" "${workers}" <<'PY'
import concurrent.futures
import sys
import time
import urllib.request

base, auth, n_s, out_path, workers_s = sys.argv[1:6]
n = int(n_s)
workers = max(1, int(workers_s))
body = b'{"model":"gpt-4o","stream":true,"messages":[{"role":"user","content":"hi"}]}'
url = base.rstrip("/") + "/v1/chat/completions"

def one(_i: int) -> float:
    headers = {
        "Content-Type": "application/json",
        "Accept": "text/event-stream",
    }
    if auth:
        headers["Authorization"] = auth
    req = urllib.request.Request(url, data=body, headers=headers, method="POST")
    t0 = time.perf_counter()
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            chunk = resp.read(1)
            ttfb = (time.perf_counter() - t0) * 1000.0
            if not chunk:
                raise RuntimeError("empty body")
            while resp.read(65536):
                pass
            return ttfb
    except Exception as e:
        raise RuntimeError(f"request failed: {e}") from e

errs = []
vals = []
with concurrent.futures.ThreadPoolExecutor(max_workers=workers) as ex:
    futs = [ex.submit(one, i) for i in range(n)]
    for f in concurrent.futures.as_completed(futs):
        try:
            vals.append(f.result())
        except Exception as e:
            errs.append(str(e))

if errs:
    sys.stderr.write("errors (%d):\n" % len(errs))
    for e in errs[:5]:
        sys.stderr.write("  %s\n" % e)
    if len(errs) > 5:
        sys.stderr.write("  ...\n")
    sys.exit(2)

vals.sort()
with open(out_path, "w", encoding="utf-8") as f:
    for v in vals:
        f.write(f"{v:.3f}\n")
print(f"ok n={len(vals)} workers={workers}", file=sys.stderr)
PY
}

percentile() {
  local file="$1" p="$2"
  python3 - "${file}" "${p}" <<'PY'
import sys
path, p_s = sys.argv[1], sys.argv[2]
p = float(p_s)
vals = [float(x) for x in open(path) if x.strip()]
if not vals:
    raise SystemExit("no samples")
vals.sort()
# nearest-rank
k = max(0, min(len(vals) - 1, int(round(p / 100.0 * (len(vals) - 1)))))
print(f"{vals[k]:.3f}")
PY
}

mean_of() {
  python3 - "$1" <<'PY'
import sys
vals = [float(x) for x in open(sys.argv[1]) if x.strip()]
print(f"{(sum(vals)/len(vals)):.3f}")
PY
}

main() {
  log "workdir=${WORKDIR}"
  log "concurrency=${CONCURRENCY} seq_samples=${SEQ_SAMPLES} slo_added_p95_ms=${SLO_ADDED_P95_MS} max_ttfb_ms=${SLO_MAX_TTFB_MS}"
  ensure_bin
  start_mock
  start_tagw
  create_member_key

  # Warmup (not measured) — fills http client pools; argon2 still paid each time.
  log "warmup ${WARMUP} requests via gateway..."
  measure_ttfb_batch "${GW_BASE}" "Bearer ${MEMBER_KEY}" "${WARMUP}" "${WORKDIR}/warm.txt" 1 \
    || die "warmup failed"
  measure_ttfb_batch "${MOCK_BASE}" "" "${WARMUP}" "${WORKDIR}/warm_mock.txt" 1 \
    || die "mock warmup failed"

  # Sequential hop estimate (fairer than concurrent against a pure-Python mock).
  log "sequential baseline: ${SEQ_SAMPLES} → mock..."
  measure_ttfb_batch "${MOCK_BASE}" "" "${SEQ_SAMPLES}" "${WORKDIR}/seq_base.txt" 1 \
    || die "sequential baseline failed"
  log "sequential gateway: ${SEQ_SAMPLES} → tagw → mock..."
  measure_ttfb_batch "${GW_BASE}" "Bearer ${MEMBER_KEY}" "${SEQ_SAMPLES}" "${WORKDIR}/seq_gw.txt" 1 \
    || die "sequential gateway failed"

  # Concurrent stability (all must succeed; stall guard on max TTFB).
  log "concurrent: ${CONCURRENCY} → mock..."
  measure_ttfb_batch "${MOCK_BASE}" "" "${CONCURRENCY}" "${WORKDIR}/c_base.txt" "${CONCURRENCY}" \
    || die "concurrent baseline failed"
  log "concurrent: ${CONCURRENCY} → tagw → mock..."
  measure_ttfb_batch "${GW_BASE}" "Bearer ${MEMBER_KEY}" "${CONCURRENCY}" "${WORKDIR}/c_gw.txt" "${CONCURRENCY}" \
    || die "concurrent gateway failed"

  local seq_base_p50 seq_base_p95 seq_base_p99 seq_gw_p50 seq_gw_p95 seq_gw_p99
  seq_base_p50="$(percentile "${WORKDIR}/seq_base.txt" 50)"
  seq_base_p95="$(percentile "${WORKDIR}/seq_base.txt" 95)"
  seq_base_p99="$(percentile "${WORKDIR}/seq_base.txt" 99)"
  seq_gw_p50="$(percentile "${WORKDIR}/seq_gw.txt" 50)"
  seq_gw_p95="$(percentile "${WORKDIR}/seq_gw.txt" 95)"
  seq_gw_p99="$(percentile "${WORKDIR}/seq_gw.txt" 99)"

  local c_base_p95 c_gw_p50 c_gw_p95 c_gw_p99 c_gw_max
  c_base_p95="$(percentile "${WORKDIR}/c_base.txt" 95)"
  c_gw_p50="$(percentile "${WORKDIR}/c_gw.txt" 50)"
  c_gw_p95="$(percentile "${WORKDIR}/c_gw.txt" 95)"
  c_gw_p99="$(percentile "${WORKDIR}/c_gw.txt" 99)"
  c_gw_max="$(python3 -c "print(f'{max(float(x) for x in open(\"${WORKDIR}/c_gw.txt\")):.3f}')")"

  local added_p50 added_p95 added_p99
  added_p50="$(python3 -c "print(f'{float('${seq_gw_p50}')-float('${seq_base_p50}'):.3f}')")"
  added_p95="$(python3 -c "print(f'{float('${seq_gw_p95}')-float('${seq_base_p95}'):.3f}')")"
  added_p99="$(python3 -c "print(f'{float('${seq_gw_p99}')-float('${seq_base_p99}'):.3f}')")"

  cat <<EOF

======== tagw SLO smoke results ========
concurrency:     ${CONCURRENCY}
seq_samples:     ${SEQ_SAMPLES}
mock base:       ${MOCK_BASE}
gateway:         ${GW_BASE}
binary:          ${BIN}

--- sequential (hop estimate; workers=1) ---
                    p50       p95       p99
mock TTFB ms     ${seq_base_p50}    ${seq_base_p95}    ${seq_base_p99}
gateway TTFB ms  ${seq_gw_p50}    ${seq_gw_p95}    ${seq_gw_p99}
added (gw-base)  ${added_p50}    ${added_p95}    ${added_p99}

--- concurrent (stability; workers=${CONCURRENCY}) ---
mock p95 TTFB ms:    ${c_base_p95}
gateway p50/p95/p99: ${c_gw_p50} / ${c_gw_p95} / ${c_gw_p99}
gateway max TTFB ms: ${c_gw_max}

assert: sequential added p95 < ${SLO_ADDED_P95_MS} ms (client-side; includes argon2 auth)
assert: concurrent gateway max TTFB < ${SLO_MAX_TTFB_MS} ms (stall guard)
assert: all ${CONCURRENCY} concurrent streams succeed
design hop target: added p95 < 10 ms *post-auth* (proxy hop; see ADR + design §4)
========================================
EOF

  python3 - <<PY
added_p95 = float("${added_p95}")
gw_max = float("${c_gw_max}")
budget = float("${SLO_ADDED_P95_MS}")
stall = float("${SLO_MAX_TTFB_MS}")
ok = True
if added_p95 >= budget:
    print(f"FAIL: sequential added p95 {added_p95:.3f} ms >= budget {budget:.3f} ms")
    ok = False
if gw_max >= stall:
    print(f"FAIL: concurrent max gateway TTFB {gw_max:.3f} ms >= stall budget {stall:.3f} ms")
    ok = False
if not ok:
    raise SystemExit(1)
print(
    f"PASS: seq added p95 {added_p95:.3f} ms < {budget:.3f} ms; "
    f"concurrent max {gw_max:.3f} ms < {stall:.3f} ms; "
    f"{int('${CONCURRENCY}')} concurrent streams ok"
)
PY
}

main "$@"
