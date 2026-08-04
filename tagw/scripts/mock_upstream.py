#!/usr/bin/env python3
"""Minimal streaming mock upstream for tagw SLO smoke.

Serves OpenAI-compatible POST /v1/chat/completions as SSE with optional
inter-chunk delay. First byte is sent immediately so TTFB measures hop cost.

Usage:
  python3 mock_upstream.py [--port 0] [--chunk-delay-ms 5] [--port-file PATH]
"""

from __future__ import annotations

import argparse
import sys
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


class FastBindHTTPServer(ThreadingHTTPServer):
    """Avoid socket.getfqdn() reverse-DNS hang in server_bind (common on macOS)."""

    def server_bind(self) -> None:
        self.socket.bind(self.server_address)
        self.server_address = self.socket.getsockname()
        host, port = self.server_address[:2]
        self.server_name = str(host)
        self.server_port = int(port)


class Handler(BaseHTTPRequestHandler):
    chunk_delay_ms: float = 5.0

    def log_message(self, fmt: str, *args) -> None:  # quieter
        sys.stderr.write("%s - %s\n" % (self.address_string(), fmt % args))
        sys.stderr.flush()

    def do_GET(self) -> None:
        if self.path in ("/healthz", "/"):
            self.send_response(200)
            self.send_header("Content-Type", "text/plain")
            self.end_headers()
            self.wfile.write(b"ok")
            return
        self.send_error(404)

    def do_POST(self) -> None:
        path = self.path.split("?", 1)[0]
        if path != "/v1/chat/completions":
            self.send_error(404)
            return

        length = int(self.headers.get("Content-Length", "0") or "0")
        if length:
            _ = self.rfile.read(length)

        chunks = [
            b'data: {"id":"chunk-1","object":"chat.completion.chunk","choices":[{"delta":{"content":"Hello"}}]}\n\n',
            b'data: {"id":"chunk-2","object":"chat.completion.chunk","choices":[{"delta":{"content":" world"}}]}\n\n',
            b'data: {"id":"chunk-3","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":2}}\n\n',
            b"data: [DONE]\n\n",
        ]

        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.send_header("Cache-Control", "no-cache")
        self.send_header("Connection", "close")
        self.end_headers()
        self.wfile.write(chunks[0])
        self.wfile.flush()
        delay = max(0.0, float(self.chunk_delay_ms) / 1000.0)
        for c in chunks[1:]:
            if delay:
                time.sleep(delay)
            self.wfile.write(c)
            self.wfile.flush()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=0, help="0 = ephemeral")
    ap.add_argument("--chunk-delay-ms", type=float, default=5.0)
    ap.add_argument(
        "--port-file",
        default="",
        help="If set, write host:port to this path after bind (for harnesses).",
    )
    args = ap.parse_args()

    Handler.chunk_delay_ms = args.chunk_delay_ms
    try:
        httpd = FastBindHTTPServer((args.host, args.port), Handler)
    except OSError as e:
        sys.stderr.write(f"bind failed: {e}\n")
        return 1

    host, port = httpd.server_address[:2]
    line = f"LISTENING {host}:{port}"
    # Always emit to stdout (and optionally a port file for redirection-safe harnesses).
    sys.stdout.write(line + "\n")
    sys.stdout.flush()
    if args.port_file:
        with open(args.port_file, "w", encoding="utf-8") as f:
            f.write(f"{host}:{port}\n")
            f.flush()

    try:
        httpd.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        httpd.server_close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
