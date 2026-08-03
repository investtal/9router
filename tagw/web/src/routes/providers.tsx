import { createFileRoute } from '@tanstack/react-router';
import { useCallback, useEffect, useState, type FormEvent } from 'react';
import {
  API_KEY_PROVIDER_TYPES,
  createAccount,
  createProvider,
  fetchMe,
  fetchProviders,
  OAUTH_PROVIDERS,
  oauthStartUrl,
  patchAccount,
  patchProvider,
  type DashboardUser,
  type ProviderPublic,
} from '../lib/api';

export const Route = createFileRoute('/providers')({
  component: ProvidersPage,
});

function ProvidersPage() {
  const [user, setUser] = useState<DashboardUser | null>(null);
  const [providers, setProviders] = useState<ProviderPublic[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);

  const [provType, setProvType] = useState<string>('openai_compat');
  const [provName, setProvName] = useState('');

  const [acctProviderId, setAcctProviderId] = useState('');
  const [acctLabel, setAcctLabel] = useState('');
  const [acctKey, setAcctKey] = useState('');
  const [acctBase, setAcctBase] = useState('');

  const isAdmin = user?.role === 'admin';

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const [me, data] = await Promise.all([fetchMe(), fetchProviders()]);
      setUser(me);
      setProviders(data);
      setError(null);
      setAcctProviderId((prev) => {
        if (prev) return prev;
        const apiKey = data.find((p) => p.kind === 'api_key');
        return apiKey?.id ?? prev;
      });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  async function onCreateProvider(e: FormEvent) {
    e.preventDefault();
    if (!isAdmin) return;
    setBusy(true);
    try {
      await createProvider({ provider_type: provType, name: provName.trim() });
      setProvName('');
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function onCreateAccount(e: FormEvent) {
    e.preventDefault();
    if (!isAdmin || !acctProviderId) return;
    setBusy(true);
    try {
      await createAccount(acctProviderId, {
        label: acctLabel.trim(),
        api_key: acctKey,
        base_url: acctBase.trim() || null,
      });
      setAcctLabel('');
      setAcctKey('');
      setAcctBase('');
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function onToggleProvider(p: ProviderPublic) {
    if (!isAdmin) return;
    setBusy(true);
    try {
      await patchProvider(p.id, !p.enabled);
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  async function onToggleAccount(providerId: string, accountId: string, enabled: boolean) {
    if (!isAdmin) return;
    setBusy(true);
    try {
      await patchAccount(providerId, accountId, !enabled);
      await reload();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }

  const apiKeyProviders = providers.filter((p) => p.kind === 'api_key');

  return (
    <div>
      <h1>Providers</h1>
      {error ? <div className="error card">{error}</div> : null}
      {loading ? <p className="muted">Loading…</p> : null}

      {isAdmin ? (
        <>
          <div className="card">
            <strong>OAuth connect</strong>
            <p className="muted" style={{ margin: '0.35rem 0 0.75rem' }}>
              Start OAuth for a provider (admin session required). Opens the IdP authorize URL.
            </p>
            <div className="row">
              {OAUTH_PROVIDERS.map((id) => (
                <a key={id} className="btn-link" href={oauthStartUrl(id)}>
                  Connect {id}
                </a>
              ))}
            </div>
          </div>

          <form className="card" onSubmit={onCreateProvider}>
            <strong>Add API-key provider</strong>
            <div className="row" style={{ marginTop: '0.75rem' }}>
              <label>
                Type{' '}
                <select value={provType} onChange={(e) => setProvType(e.target.value)}>
                  {API_KEY_PROVIDER_TYPES.map((t) => (
                    <option key={t} value={t}>
                      {t}
                    </option>
                  ))}
                </select>
              </label>
              <label>
                Name{' '}
                <input
                  value={provName}
                  onChange={(e) => setProvName(e.target.value)}
                  placeholder="My GLM"
                  required
                />
              </label>
              <button type="submit" disabled={busy || !provName.trim()}>
                Create provider
              </button>
            </div>
          </form>

          <form className="card" onSubmit={onCreateAccount}>
            <strong>Add account</strong>
            <div className="row" style={{ marginTop: '0.75rem' }}>
              <label>
                Provider{' '}
                <select
                  value={acctProviderId}
                  onChange={(e) => setAcctProviderId(e.target.value)}
                  required
                >
                  <option value="" disabled>
                    Select…
                  </option>
                  {apiKeyProviders.map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.name} ({p.provider_type})
                    </option>
                  ))}
                </select>
              </label>
              <label>
                Label{' '}
                <input
                  value={acctLabel}
                  onChange={(e) => setAcctLabel(e.target.value)}
                  placeholder="primary"
                  required
                />
              </label>
              <label>
                API key{' '}
                <input
                  value={acctKey}
                  onChange={(e) => setAcctKey(e.target.value)}
                  placeholder="sk-…"
                  required
                  autoComplete="off"
                />
              </label>
              <label>
                Base URL{' '}
                <input
                  value={acctBase}
                  onChange={(e) => setAcctBase(e.target.value)}
                  placeholder="optional (required for openai_compat)"
                />
              </label>
              <button
                type="submit"
                disabled={busy || !acctProviderId || !acctLabel.trim() || !acctKey}
              >
                Add account
              </button>
            </div>
          </form>
        </>
      ) : null}

      {providers.length === 0 && !loading ? (
        <p className="muted">No providers configured.</p>
      ) : null}
      {providers.map((p) => (
        <div className="card" key={p.id}>
          <div className="row" style={{ justifyContent: 'space-between' }}>
            <div>
              <strong>{p.name}</strong>{' '}
              <span className="badge">{p.provider_type}</span>{' '}
              <span className="badge">{p.kind}</span>
            </div>
            <div className="row" style={{ marginBottom: 0 }}>
              <span className={`badge ${p.enabled ? 'on' : 'off'}`}>
                {p.enabled ? 'enabled' : 'disabled'}
              </span>
              {isAdmin ? (
                <button
                  type="button"
                  className="secondary"
                  disabled={busy}
                  onClick={() => void onToggleProvider(p)}
                >
                  {p.enabled ? 'Disable' : 'Enable'}
                </button>
              ) : null}
            </div>
          </div>
          <div className="muted mono" style={{ fontSize: '0.8rem' }}>
            {p.id}
          </div>
          {p.accounts.length === 0 ? (
            <p className="muted">No accounts</p>
          ) : (
            <table style={{ marginTop: '0.75rem' }}>
              <thead>
                <tr>
                  <th>Label</th>
                  <th>Key prefix</th>
                  <th>Base URL</th>
                  <th>Status</th>
                  {isAdmin ? <th /> : null}
                </tr>
              </thead>
              <tbody>
                {p.accounts.map((a) => (
                  <tr key={a.id}>
                    <td>{a.label}</td>
                    <td className="mono">{a.credentials.api_key_prefix}</td>
                    <td className="mono">{a.credentials.base_url ?? '—'}</td>
                    <td>
                      <span className={`badge ${a.enabled ? 'on' : 'off'}`}>
                        {a.enabled ? 'on' : 'off'}
                      </span>
                    </td>
                    {isAdmin ? (
                      <td>
                        <button
                          type="button"
                          className="secondary"
                          disabled={busy}
                          onClick={() => void onToggleAccount(p.id, a.id, a.enabled)}
                        >
                          {a.enabled ? 'Disable' : 'Enable'}
                        </button>
                      </td>
                    ) : null}
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      ))}
    </div>
  );
}
