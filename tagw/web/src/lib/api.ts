/** Shared API client for the TAGW dashboard (cookie session). */

export type Range = 'today' | '3d' | '7d' | '30d' | '90d';

export const RANGES: Range[] = ['today', '3d', '7d', '30d', '90d'];

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

/** Build query string for a usage range filter. */
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
