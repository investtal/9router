#!/bin/bash
# monitor-mitm-ubuntu.sh
# Simple monitoring script to validate P1-01 (and general MITM stability) on Ubuntu
# Usage: ./scripts/monitor-mitm-ubuntu.sh [interval_seconds]
#
# Run this in a separate terminal while doing heavy MITM usage (Copilot + Cursor + long coding sessions)
# It will log:
#   - MITM process tree (wrapper vs real child)
#   - Memory (RSS) of relevant node processes
#   - Open file descriptors for the MITM child
#   - /etc/hosts pollution related to 9router
#   - Orphan detection

set -euo pipefail

INTERVAL=${1:-5}
DATA_DIR="${DATA_DIR:-$HOME/.9router}"
MITM_PID_FILE="$DATA_DIR/mitm/.mitm.pid"
HOSTS_FILE="/etc/hosts"

echo "=== 9Router MITM Ubuntu Monitor (P1-01 validation) ==="
echo "Interval: ${INTERVAL}s | Data dir: $DATA_DIR"
echo "Press Ctrl+C to stop. Log will also be written to /tmp/9router-mitm-monitor.log"
echo ""

exec > >(tee -a /tmp/9router-mitm-monitor.log) 2>&1

while true; do
  echo "========== $(date '+%Y-%m-%d %H:%M:%S') =========="

  # 1. Find MITM-related node processes
  echo "--- MITM Process Tree ---"
  ps aux | grep -E 'node.*server.js|mitm' | grep -v grep || echo "No MITM node processes found"

  # 2. If we have a PID file, show details
  if [[ -f "$MITM_PID_FILE" ]]; then
    SAVED_PID=$(cat "$MITM_PID_FILE" 2>/dev/null || echo "0")
    echo "Saved PID from file: $SAVED_PID"

    if [[ "$SAVED_PID" =~ ^[0-9]+$ ]] && kill -0 "$SAVED_PID" 2>/dev/null; then
      echo "Process $SAVED_PID is alive (according to kill -0)"
      ps -p "$SAVED_PID" -o pid,ppid,user,%cpu,%mem,rss,vsz,stat,start,time,cmd || true

      # Show children (important: on sudo spawn, the real node is often a child of the wrapper)
      echo "Direct children of $SAVED_PID:"
      pgrep -P "$SAVED_PID" -a || echo "  (no direct children or pgrep failed)"

      # Memory in MB
      RSS_KB=$(ps -p "$SAVED_PID" -o rss= 2>/dev/null || echo 0)
      echo "RSS: $(( RSS_KB / 1024 )) MB"

      # Open file descriptors (critical for P1-01 validation + leak detection)
      if [[ -d "/proc/$SAVED_PID/fd" ]]; then
        FD_COUNT=$(ls /proc/$SAVED_PID/fd 2>/dev/null | wc -l || echo "?")
        echo "Open FDs for $SAVED_PID: $FD_COUNT"
      fi
    else
      echo "Saved PID $SAVED_PID is dead or not accessible"
    fi
  else
    echo "No MITM PID file at $MITM_PID_FILE"
  fi

  # 3. Look for any root-owned node processes that might be orphaned MITM children
  echo "--- Potential orphaned root node processes ---"
  ps -u root -o pid,ppid,cmd 2>/dev/null | grep -E 'node.*server.js|9router' || echo "None found"

  # 4. Check /etc/hosts for leftover 9router/tool domains
  echo "--- /etc/hosts pollution check ---"
  grep -E 'cloudcode-pa|githubcopilot|kiro|antigravity' "$HOSTS_FILE" 2>/dev/null | head -10 || echo "No obvious MITM entries in hosts"

  # 5. Port 443 listener
  echo "--- Port 443 listeners ---"
  ss -tlnp 2>/dev/null | grep ':443 ' || lsof -i :443 2>/dev/null | head -5 || echo "Nothing listening on 443 or tools unavailable"

  echo ""
  sleep "$INTERVAL"
done
