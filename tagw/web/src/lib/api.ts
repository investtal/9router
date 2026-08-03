/** Shared API client for the TAGW dashboard (cookie session). */

export type Range = 'today' | '3d' | '7d' | '30d' | '90d';

export const RANGES: Range[] = ['today', '3d', '7d', '30d', '90d'];

export const API_KEY_PROVIDER_TYPES = [
  'glm',
  'open_model',
  'alibaba',
  'anthropic',
  'minimax',
  'kimi',
  'deepseek',
  'openai_compat',
] as const;

export type ApiKeyProviderType = (typeof API_KEY_PROVIDER_TYPES)[number];

export const OAUTH_PROVIDERS = ['codex', 'claude', 'xai', 'kimi', 'antigravity'] as const;

export type DashboardUser = {
  id: string;
  username: string;
  role: 'viewer' | 'admin';
};

export type UsageOverview = {
  range: string;
  from: string;
  to: string;
  request_count: number;
  prompt_tokens: number;
  completion_tokens: number;
  cached_tokens: number;
  cost_est: number;
};

export type RequestLogRow = {
  id: string;
  created_at: string;
  member_key_id: string | null;
  provider_id: string | null;
  account_id: string | null;
  model: string | null;
  tool: string | null;
  status: number | null;
  prompt_tokens: number;
  completion_tokens: number;
  cached_tokens: number;
  cost_est: number;
  latency_ms: number | null;
  ttft_ms: number | null;
  usage_incomplete: boolean;
  error: string | null;
};

export type RequestListResponse = {
  items: RequestLogRow[];
  next_cursor: string | null;
};

export type MemberModelCell = {
  member_key_id: string;
  member_name: string | null;
  model: string;
  request_count: number;
  prompt_tokens: number;
  completion_tokens: number;
  cached_tokens: number;
  cost_est: number;
};

export type ProviderPublic = {
  id: string;
  kind: string;
  provider_type: string;
  name: string;
  enabled: boolean;
  config_json: unknown;
  created_at: string;
  accounts: AccountPublic[];
};

export type AccountPublic = {
  id: string;
  provider_id: string;
  label: string;
  enabled: boolean;
  credentials: {
    api_key_prefix: string;
    base_url?: string | null;
    models?: string[] | null;
  };
  quota_json: unknown;
  created_at: string;
};

export type MemberApiKeyPublic = {
  id: string;
  name: string;
  key_prefix: string;
  created_at: string;
  revoked_at: string | null;
};

export type CreateKeyResponse = MemberApiKeyPublic & {
  key: string;
};

export type LiveEvent = {
  id: string;
  ts: string;
  level: string;
  message: string;
  request_id: string | null;
  member_key_id: string | null;
  model: string | null;
};

export type CreateProviderInput = {
  provider_type: string;
  name: string;
  enabled?: boolean;
};

export type CreateAccountInput = {
  label: string;
  api_key: string;
  base_url?: string | null;
  models?: string[] | null;
  enabled?: boolean;
};

export type OAuthStartResponse = {
  provider: string;
  authorize_url: string;
  state: string;
  redirect_uri: string;
};

export type ExportBundle = {
  version: number;
  exported_at: string;
  providers: unknown[];
  accounts: unknown[];
  users: unknown[];
  member_api_keys: unknown[];
  settings: unknown;
  include_request_logs?: boolean;
  request_logs?: unknown[];
};

export type ImportResult = {
  providers: number;
  accounts: number;
  users: number;
  member_api_keys: number;
  settings: number;
  request_logs: number;
};

export type UserPublic = {
  id: string;
  username: string;
  role: 'viewer' | 'admin';
  created_at: string;
};

export type CreateUserInput = {
  username: string;
  password: string;
  role?: 'viewer' | 'admin';
};

export function rangeQuery(range: Range): string {
  return `range=${encodeURIComponent(range)}`;
}

async function apiFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const r = await fetch(path, {
    ...init,
    credentials: 'include',
    headers: {
      Accept: 'application/json',
      ...(init?.body ? { 'Content-Type': 'application/json' } : {}),
      ...init?.headers,
    },
  });
  if (!r.ok) {
    const text = await r.text();
    throw new Error(text || `${r.status} ${r.statusText}`);
  }
  if (r.status === 204) {
    return undefined as T;
  }
  return r.json() as Promise<T>;
}

export async function login(username: string, password: string): Promise<DashboardUser> {
  return apiFetch<DashboardUser>('/api/auth/login', {
    method: 'POST',
    body: JSON.stringify({ username, password }),
  });
}

export async function logout(): Promise<void> {
  await apiFetch<{ ok: boolean }>('/api/auth/logout', { method: 'POST' });
}

export async function fetchMe(): Promise<DashboardUser> {
  return apiFetch<DashboardUser>('/api/auth/me');
}

export async function fetchOverview(range: Range): Promise<UsageOverview> {
  return apiFetch<UsageOverview>(`/api/usage/overview?${rangeQuery(range)}`);
}

export async function fetchRequests(params: {
  limit?: number;
  member_key_id?: string;
  model?: string;
}): Promise<RequestListResponse> {
  const q = new URLSearchParams();
  if (params.limit != null) q.set('limit', String(params.limit));
  if (params.member_key_id) q.set('member_key_id', params.member_key_id);
  if (params.model) q.set('model', params.model);
  const qs = q.toString();
  return apiFetch<RequestListResponse>(`/api/usage/requests${qs ? `?${qs}` : ''}`);
}

export async function fetchMembers(range: Range): Promise<MemberModelCell[]> {
  return apiFetch<MemberModelCell[]>(`/api/usage/members?${rangeQuery(range)}`);
}

export async function fetchProviders(): Promise<ProviderPublic[]> {
  return apiFetch<ProviderPublic[]>('/api/providers');
}

export async function createProvider(input: CreateProviderInput): Promise<ProviderPublic> {
  return apiFetch<ProviderPublic>('/api/admin/providers', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export async function patchProvider(
  id: string,
  enabled: boolean,
): Promise<{ id: string; enabled: boolean }> {
  return apiFetch(`/api/admin/providers/${encodeURIComponent(id)}`, {
    method: 'PATCH',
    body: JSON.stringify({ enabled }),
  });
}

export async function createAccount(
  providerId: string,
  input: CreateAccountInput,
): Promise<AccountPublic> {
  return apiFetch<AccountPublic>(
    `/api/admin/providers/${encodeURIComponent(providerId)}/accounts`,
    {
      method: 'POST',
      body: JSON.stringify(input),
    },
  );
}

export async function patchAccount(
  providerId: string,
  accountId: string,
  enabled: boolean,
): Promise<{ id: string; provider_id: string; enabled: boolean }> {
  return apiFetch(
    `/api/admin/providers/${encodeURIComponent(providerId)}/accounts/${encodeURIComponent(accountId)}`,
    {
      method: 'PATCH',
      body: JSON.stringify({ enabled }),
    },
  );
}

/** Fetch OAuth authorize URL (JSON; does not auto-redirect). */
export async function startOAuth(provider: string): Promise<OAuthStartResponse> {
  return apiFetch<OAuthStartResponse>(
    `/api/oauth/${encodeURIComponent(provider)}/start?redirect=false`,
  );
}

/** Browser navigation URL that 302s to the IdP (session cookie sent). */
export function oauthStartUrl(provider: string): string {
  return `/api/oauth/${encodeURIComponent(provider)}/start`;
}

export async function exportBundle(): Promise<ExportBundle> {
  return apiFetch<ExportBundle>('/api/admin/export/bundle');
}

export async function importBundle(bundle: unknown): Promise<ImportResult> {
  return apiFetch<ImportResult>('/api/admin/import/bundle', {
    method: 'POST',
    body: JSON.stringify(bundle),
  });
}

/** Same-origin DB file download (uses session cookie via navigation / anchor). */
export function exportDbUrl(): string {
  return '/api/admin/export/db';
}

export async function listUsers(): Promise<UserPublic[]> {
  return apiFetch<UserPublic[]>('/api/admin/users');
}

export async function createUser(input: CreateUserInput): Promise<UserPublic> {
  return apiFetch<UserPublic>('/api/admin/users', {
    method: 'POST',
    body: JSON.stringify(input),
  });
}

export async function fetchAdminKeys(): Promise<MemberApiKeyPublic[]> {
  return apiFetch<MemberApiKeyPublic[]>('/api/admin/keys');
}

export async function createAdminKey(name: string): Promise<CreateKeyResponse> {
  return apiFetch<CreateKeyResponse>('/api/admin/keys', {
    method: 'POST',
    body: JSON.stringify({ name }),
  });
}

export async function revokeAdminKey(id: string): Promise<void> {
  await apiFetch<void>(`/api/admin/keys/${encodeURIComponent(id)}`, {
    method: 'DELETE',
  });
}

export async function fetchRecentLogs(limit = 100): Promise<LiveEvent[]> {
  return apiFetch<LiveEvent[]>(`/api/logs/recent?limit=${limit}`);
}
