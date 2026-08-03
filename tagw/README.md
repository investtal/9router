# tagw — Team AI Gateway

Single Rust binary: OpenAI-compatible + Anthropic streaming proxy, admin REST, OAuth, live logs, and TanStack SPA dashboard.

Default bind: **`0.0.0.0:20128`** (same port story as 9router; change with `TAGW_BIND` if both run).

## Quick start

```bash
# 1) Build the gateway
cd tagw
cargo build --release -p tagw

# 2) (Optional) build dashboard assets — required for SPA from the binary
cd web && npm install && npm run build && cd ..

# 3) Run (dev upstream optional — without it, /v1 returns 502 until providers are configured)
export TAGW_DATA_DIR=./data
export TAGW_SESSION_SECRET="$(openssl rand -hex 32)"   # set in production
export TAGW_ADMIN_PASSWORD='change-me'                # first boot only (seed)
# Optional dev fallback when account pool is empty:
# export TAGW_UPSTREAM=https://api.openai.com
# export TAGW_UPSTREAM_AUTH='Bearer sk-...'

./target/release/tagw
```

Open **http://127.0.0.1:20128/** (SPA) or **http://127.0.0.1:20128/login**.

### Default login (first empty DB)

| Field | Value |
|-------|--------|
| Username | `admin` |
| Password | `admin` (or `TAGW_ADMIN_PASSWORD` if set **before** first migrate/seed) |

Change the password for any real deployment. Seed runs only when `users` is empty.

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `TAGW_BIND` | `0.0.0.0:20128` | Listen address |
| `TAGW_DATA_DIR` | `./data` | SQLite dir (`gateway.db`) |
| `TAGW_SESSION_SECRET` | insecure dev default | HMAC secret for `tagw_session` cookie |
| `TAGW_ADMIN_PASSWORD` | `admin` | Password for seeded admin user (empty DB only) |
| `TAGW_UPSTREAM` | unset | Dev fallback upstream base when provider pool empty |
| `TAGW_UPSTREAM_AUTH` | unset | Optional `Authorization` value for fallback upstream |
| `TAGW_PUBLIC_BASE` | unset | Public URL for OAuth `redirect_uri` (e.g. `http://127.0.0.1:20128`) |
| `TAGW_WEB_DIR` | `tagw/web/dist` | Built SPA assets directory |
| `RUST_LOG` | unset | e.g. `tagw=info,tower_http=info` |

OIDC (optional Keycloak): see code under `auth/oidc.rs` and settings once configured via admin/DB.

## Dashboard flow

1. Log in at `/login` (basic auth cookie session).
2. **Providers** — add API-key accounts and/or connect OAuth.
3. **Admin → Keys** — create a **member API key** (`sk-…`). Copy once; only prefix is stored for list UI.
4. Point clients at the gateway with that member key (not the upstream provider key).

Roles: **admin** can manage keys/providers/users/import-export; **viewer** is read-mostly (403 on admin mutations).

## Client configuration

Replace `GATEWAY` with your base (e.g. `http://127.0.0.1:20128`). Use a **member API key** from the dashboard.

### Codex (OpenAI-compatible)

```bash
export OPENAI_BASE_URL="http://GATEWAY/v1"
export OPENAI_API_KEY="sk-your-member-key"
# Then run Codex / any OpenAI SDK client against OPENAI_BASE_URL
```

### pi (OpenAI-compatible)

```bash
# Same shape as Codex — OpenAI base URL + member key
export OPENAI_BASE_URL="http://GATEWAY/v1"
export OPENAI_API_KEY="sk-your-member-key"
```

### Claude Code

**Anthropic Messages path** (native):

```bash
export ANTHROPIC_BASE_URL="http://GATEWAY"
export ANTHROPIC_API_KEY="sk-your-member-key"
# Claude Code / SDKs that call POST /v1/messages
```

**OpenAI-compatible mode** (if your Claude Code build supports a custom OpenAI base):

```bash
export OPENAI_BASE_URL="http://GATEWAY/v1"
export OPENAI_API_KEY="sk-your-member-key"
```

Gateway routes of interest:

| Route | Purpose |
|-------|---------|
| `POST /v1/chat/completions` | OpenAI-compatible chat (streaming) |
| `POST /v1/*` | Other OpenAI-shaped paths (proxy passthrough) |
| `POST /v1/messages` | Anthropic Messages (Claude Code) |
| `GET /healthz` | Liveness |
| `GET /api/logs/stream` | Live console SSE (dashboard session) |

## Develop

```bash
# API + unit/integration tests
cd tagw && cargo test -p tagw

# SPA dev server (proxies /api and /v1 → :20128)
cd tagw/web && npm run dev

# SLO smoke (mock upstream + 50 concurrent streams)
./tagw/scripts/slo_smoke.sh
```

## Import / export

Admin (session cookie) endpoints:

- `GET /api/admin/export/db` — raw SQLite file
- `GET /api/admin/export/bundle` — JSON bundle (providers, keys metadata, usage, …)
- `POST /api/admin/import/bundle` — restore bundle

Typical backup: export DB or bundle → wipe `TAGW_DATA_DIR` → restore → confirm usage history.

## Architecture notes

- **HTTP stack:** axum/hyper for v1 — see [`docs/adr/2026-08-03-proxy-http-stack.md`](../docs/adr/2026-08-03-proxy-http-stack.md).
- **Routing:** round-robin + pre-body fail-over across provider accounts (`AccountRouter`).
- **Secrets:** member keys argon2-hashed; provider secrets redacted in list APIs; `AppError::Internal` never returns raw details to clients.
- **Usage:** async write queue → SQLite WAL.

## Manual v1 checklist (operators)

- [ ] Codex → `GATEWAY/v1` + member key
- [ ] pi same
- [ ] Claude Code Anthropic base URL and/or OpenAI mode
- [ ] RR across 2 API keys for one provider
- [ ] Kill one upstream (429) → fail-over
- [ ] OAuth refresh survives process restart (tokens in SQLite)
- [ ] Viewer cannot create keys; admin can
- [ ] Export DB, wipe data dir, restore, usage history present
- [ ] Live logs show requests within ~1s
- [ ] No secrets in client-visible errors (`cargo test -p tagw` covers `AppError::Internal` redaction)
