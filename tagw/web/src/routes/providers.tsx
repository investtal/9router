import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { fetchProviders, type ProviderPublic } from '../lib/api';

export const Route = createFileRoute('/providers')({
  component: ProvidersPage,
});

function ProvidersPage() {
  const [providers, setProviders] = useState<ProviderPublic[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    fetchProviders()
      .then((data) => {
        if (!cancelled) {
          setProviders(data);
          setError(null);
        }
      })
      .catch((err) => {
        if (!cancelled) setError(err instanceof Error ? err.message : String(err));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div>
      <h1>Providers</h1>
      {error ? <div className="error card">{error}</div> : null}
      {loading ? <p className="muted">Loading…</p> : null}
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
            <span className={`badge ${p.enabled ? 'on' : 'off'}`}>
              {p.enabled ? 'enabled' : 'disabled'}
            </span>
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
