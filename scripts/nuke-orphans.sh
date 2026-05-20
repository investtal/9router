#!/bin/bash
# nuke-orphans.sh
# Standalone "nuke leftovers" helper for 9Router on Linux/macOS.
#
# Use this when:
#   - Port 443 is stuck ("Address already in use")
#   - Multiple 9router processes are running after a crash
#   - MITM refuses to start
#   - You see ghost cloudflared / tailscale / tray processes
#
# This is deliberately conservative and noisy.
# It will NEVER do a broad "kill all node" or "pkill 9router".
#
# Usage:
#   ./scripts/nuke-orphans.sh           # interactive (asks for confirmation)
#   ./scripts/nuke-orphans.sh --force   # non-interactive, still very conservative
#
# Safe to run even if 9Router is currently running (it skips your own PID).

set -euo pipefail

FORCE=0
if [[ "${1:-}" == "--force" ]]; then
  FORCE=1
fi

echo "=== 9Router Orphan Nuke Script (P1-05 standalone helper) ==="
echo "Date: $(date)"
echo "User: $(whoami)"
echo ""

# --- Helpers ---------------------------------------------------------

log()  { echo "[nuke] $*"; }
warn() { echo "[nuke] WARNING: $*" >&2; }

is_alive() {
  kill -0 "$1" 2>/dev/null
}

# Conservative pattern list (very specific)
PATTERNS=(
  "node.*9router.*cli\\.js"
  "node.*9router.*server\\.js"
  "node.*9router.*mitm.*server\\.js"
  "cloudflared.*9router"
  "tailscaled.*9router"
  "tray_linux_release"
)

# --- Step 1: Clean known stale PID files -----------------------------

DATA_DIR="${DATA_DIR:-$HOME/.9router}"
MITM_PID="$DATA_DIR/mitm/.mitm.pid"
CLOUDFLARED_PID="$DATA_DIR/tunnel/cloudflared.pid"
TAILSCALE_PID="$DATA_DIR/tunnel/tailscale.pid"

for pidfile in "$MITM_PID" "$CLOUDFLARED_PID" "$TAILSCALE_PID"; do
  if [[ -f "$pidfile" ]]; then
    pid=$(cat "$pidfile" 2>/dev/null | tr -d ' \n' || echo "")
    if [[ -n "$pid" && ! "$pid" =~ ^[0-9]+$ ]]; then
      warn "Bad content in $pidfile, removing"
      rm -f "$pidfile"
      continue
    fi
    if [[ -n "$pid" ]] && ! is_alive "$pid"; then
      log "Removing stale PID file: $pidfile (process $pid is dead)"
      rm -f "$pidfile"
    fi
  fi
done

# --- Step 2: Find candidate processes (very conservative) ------------

CANDIDATES=()

if [[ "$(uname)" != "Linux" && "$(uname)" != "Darwin" ]]; then
  echo "This script currently only supports Linux and macOS."
  exit 1
fi

for pattern in "${PATTERNS[@]}"; do
  pids=$(pgrep -f "$pattern" 2>/dev/null || true)
  for pid in $pids; do
    [[ "$pid" == "$$" ]] && continue          # ourselves
    [[ "$pid" == "$PPID" ]] && continue

    # Extra safety: re-check cmdline contains "9router" + one of our markers
    cmd=$(ps -p "$pid" -o cmd= 2>/dev/null | tr '[:upper:]' '[:lower:]' || true)
    if echo "$cmd" | grep -q "9router" && \
       ( echo "$cmd" | grep -qE "cli\.js|server\.js|mitm" ); then
      CANDIDATES+=("$pid|$cmd")
    fi
  done
done

# Also look for root-owned MITM children that might have been missed
if command -v pgrep >/dev/null 2>&1; then
  root_mitm=$(pgrep -u root -f "node.*server\.js" 2>/dev/null || true)
  for pid in $root_mitm; do
    cmd=$(ps -p "$pid" -o cmd= 2>/dev/null | tr '[:upper:]' '[:lower:]' || true)
    if echo "$cmd" | grep -q "9router.*mitm"; then
      CANDIDATES+=("$pid|$cmd (root)")
    fi
  done
fi

# Deduplicate
CANDIDATES=($(printf "%s\n" "${CANDIDATES[@]}" | sort -u))

if [[ ${#CANDIDATES[@]} -eq 0 ]]; then
  log "No obvious 9Router orphan processes found. Environment looks clean."
  exit 0
fi

echo ""
echo "Found the following candidate orphan processes:"
echo "------------------------------------------------"
for entry in "${CANDIDATES[@]}"; do
  pid="${entry%%|*}"
  cmd="${entry#*|}"
  echo "  PID $pid : $cmd"
done
echo "------------------------------------------------"

# --- Step 3: Confirmation / Action -----------------------------------

if [[ $FORCE -eq 0 ]]; then
  echo ""
  read -r -p "Kill the processes above with SIGTERM first? [y/N] " reply
  if [[ ! "$reply" =~ ^[Yy]$ ]]; then
    echo "Aborted by user."
    exit 0
  fi
else
  log "--force mode: proceeding without prompt"
fi

for entry in "${CANDIDATES[@]}"; do
  pid="${entry%%|*}"
  cmd="${entry#*|}"

  log "Sending SIGTERM to PID $pid"
  if [[ "$cmd" == *"(root)"* ]]; then
    sudo kill -TERM "$pid" 2>/dev/null || true
  else
    kill -TERM "$pid" 2>/dev/null || true
  fi
done

sleep 1.2

# Second pass - hard kill anything still alive
for entry in "${CANDIDATES[@]}"; do
  pid="${entry%%|*}"
  if is_alive "$pid"; then
    log "Process $pid still alive after TERM → sending SIGKILL"
    if [[ "$entry" == *"(root)"* ]]; then
      sudo kill -9 "$pid" 2>/dev/null || true
    else
      kill -9 "$pid" 2>/dev/null || true
    fi
  fi
done

log "Cleanup pass finished."

# Final quick check
REMAINING=$(pgrep -f "node.*9router.*(cli|server|mitm)" 2>/dev/null | wc -l || echo 0)
if [[ "$REMAINING" -gt 0 ]]; then
  warn "Some processes may still be alive. You may need to run with sudo or reboot."
else
  log "No more matching 9Router processes found."
fi

echo ""
echo "You can now try starting 9Router again."
echo "If you still have problems, run the monitor script:"
echo "   ./scripts/monitor-mitm-ubuntu.sh"
echo ""