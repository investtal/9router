# Task 8 Report: OAuth manager + token refresh

## status: DONE

## commits
- `feat(tagw): oauth connect and auto token refresh`

## what landed

### Core types / trait (`oauth/types.rs`)
- `OAuthProvider` trait: `id`, `authorize_url`, `exchange_code`, `refresh`, `default_base_url`
- `TokenSet { access_token, refresh_token, expires_at }`
- `OAuthCredentials` in `accounts.credentials_json` for `kind=oauth`
- `Pkce` + S256 helpers (`oauth/pkce.rs`)
- Skew: refresh if expires within **120s**; background lead **5min**; loop every **60s**

### Providers (endpoints from 9router open-sse registry / oauth services)
| Module | Id | Notes |
|--------|-----|--------|
| `codex.rs` | codex | Full PKCE start + form exchange + JSON refresh (matches tokenRefresh) |
| `claude.rs` | claude | PKCE + JSON exchange/refresh |
| `antigravity.rs` | antigravity | Google OAuth + client_secret |
| `xai.rs` | xai | PKCE public client form exchange/refresh |
| `kimi.rs` | kimi | Device-authorize start URL + form refresh; exchange scaffold |

Token URLs / client ids overridable via credentials (`token_url`, `client_id`) for tests.

### Refresh
- `ensure_access_token(db, cache, account_id)` (+ `_with_client` / force for tests)
- Persists new tokens to SQLite, reloads config cache
- `spawn_oauth_refresh_loop` in `main` every 60s for near-expiry accounts

### Routes
- `GET /api/oauth/:provider/start` — PKCE in memory map; 302 to authorize (or `?redirect=false` JSON)
- `GET /api/oauth/:provider/callback` — exchange code, upsert oauth provider + account, reload pools
- `TAGW_PUBLIC_BASE` for redirect_uri construction

### Pools
- `load_oauth_account_pools` merges with api_key pools in `ConfigCache::load`
- Auth header: `Bearer {access_token}` from credentials_json

## test summary
`cargo test -p tagw`: **43 passed**, 0 failed

New (`tests/oauth_refresh.rs`):
- `expired_token_refreshes_once_and_stores_in_sqlite` — wiremock token URL; expired creds → refresh once → SQLite + cache updated; second ensure no second HTTP call
- `codex_exchange_code_with_mock`
- `oauth_start_returns_authorize_url_json` — pending state stored
- `oauth_account_loads_into_pool`
- `token_set_from_oauth_json`

Unit: `oauth::pkce::tests::pkce_challenge_is_s256`

Prior suite green: health, member keys, migrate, proxy, router, usage, api_key providers.

## concerns
- Codex real refresh uses JSON (tokenRefresh); registry lists form — we follow production refresh path.
- Kimi is primarily device-code; start URL is authorize_device (not full device poll). Callback exchange is scaffolded for code grant.
- Antigravity onboarding (project id / loadCodeAssist) not ported — tokens only.
- Admin list still redacts oauth creds via api_key parser fallback (`***`); no oauth-specific admin CRUD.
- Proxy does not yet call `ensure_access_token` on each hop — relies on background loop + pool cache; stale Bearer possible until next refresh if access expires mid-request.
- Secrets (client secrets, tokens) plaintext in SQLite (v1).
- In-memory PKCE map is process-local (lost on restart; multi-instance not shared).
