#!/usr/bin/env bash
# Load .env and run tagw (release binary preferred).
set -euo pipefail
cd "$(dirname "$0")"

ENV_FILE="${TAGW_ENV_FILE:-env.local}"
if [[ ! -f "$ENV_FILE" ]]; then
  echo "missing $ENV_FILE — create tagw/env.local with TAGW_* vars" >&2
  exit 1
fi

set -a
# shellcheck disable=SC1091
source "$ENV_FILE"
set +a

BIN=./target/release/tagw
if [[ ! -x "$BIN" ]]; then
  echo "building release binary..."
  cargo build --release -p tagw
fi

if [[ ! -d "${TAGW_WEB_DIR:-./web/dist}" ]]; then
  echo "building SPA..."
  (cd web && npm run build)
fi

mkdir -p "${TAGW_DATA_DIR:-./data}"
echo "listening on ${TAGW_BIND:-0.0.0.0:20128}"
echo "dashboard: ${TAGW_PUBLIC_BASE:-http://127.0.0.1:20128}/login"
exec "$BIN"
