import { createFileRoute } from '@tanstack/react-router';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { fetchModels, type ModelEntry } from '../lib/api';
import { formatNumber } from '../lib/format';
import { ProviderLogo } from '../lib/providerLogo';

export const Route = createFileRoute('/models')({
  component: ModelsPage,
});

function ModelsPage() {
  const [items, setItems] = useState<ModelEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [filter, setFilter] = useState('');
  const [providerFilter, setProviderFilter] = useState<string>('all');
  const [copied, setCopied] = useState<string | null>(null);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const data = await fetchModels();
      setItems(data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const providers = useMemo(() => {
    const s = new Set(items.map((m) => m.provider));
    return Array.from(s).sort();
  }, [items]);

  const filtered = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return items.filter((m) => {
      if (providerFilter !== 'all' && m.provider !== providerFilter) return false;
      if (!q) return true;
      return (
        m.id.toLowerCase().includes(q) ||
        m.name.toLowerCase().includes(q) ||
        m.provider.toLowerCase().includes(q) ||
        m.upstream_model.toLowerCase().includes(q)
      );
    });
  }, [items, filter, providerFilter]);

  async function copyId(id: string) {
    try {
      await navigator.clipboard.writeText(id);
      setCopied(id);
      window.setTimeout(() => setCopied((c) => (c === id ? null : c)), 1500);
    } catch {
      // ignore
    }
  }

  return (
    <div>
      <h1>Models</h1>
      <p className="muted" style={{ marginTop: 0 }}>
        Use these ids in client <code>model</code> fields — 9router-style{' '}
        <code>provider/model</code> (e.g. <code>glm/glm-5.2</code>, <code>xai/grok-4.5</code>).
        The gateway strips the prefix before calling upstream.
      </p>

      <div className="row" style={{ gap: '0.75rem', flexWrap: 'wrap' }}>
        <label>
          Search{' '}
          <input
            value={filter}
            placeholder="glm, grok, claude…"
            onChange={(e) => setFilter(e.target.value)}
          />
        </label>
        <label>
          Provider{' '}
          <select
            value={providerFilter}
            onChange={(e) => setProviderFilter(e.target.value)}
          >
            <option value="all">all</option>
            {providers.map((p) => (
              <option key={p} value={p}>
                {p}
              </option>
            ))}
          </select>
        </label>
        <button type="button" className="secondary" onClick={() => void reload()} disabled={loading}>
          Refresh
        </button>
      </div>

      {error && <p className="error">{error}</p>}
      {loading && <p className="muted">Loading…</p>}

      {!loading && !error && filtered.length === 0 && (
        <p className="muted">
          No models yet. Connect a provider account on <strong>Providers</strong>, then refresh.
        </p>
      )}

      {!loading && filtered.length > 0 && (
        <>
          <p className="muted">
            Showing {formatNumber(filtered.length)} of {formatNumber(items.length)} model
            {items.length === 1 ? '' : 's'} from enabled accounts.
          </p>
          <div className="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Client id</th>
                  <th>Provider</th>
                  <th>Upstream model</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {filtered.map((m) => (
                  <tr key={m.id}>
                    <td>
                      <span className="provider-chip">
                        <ProviderLogo provider={m.provider} size={18} />
                        <code>{m.id}</code>
                      </span>
                    </td>
                    <td>
                      <span className="provider-chip">
                        <ProviderLogo provider={m.provider} size={16} />
                        <span className="badge">{m.provider}</span>
                      </span>
                    </td>
                    <td className="muted">{m.upstream_model}</td>
                    <td>
                      <button type="button" className="secondary" onClick={() => void copyId(m.id)}>
                        {copied === m.id ? 'Copied' : 'Copy'}
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </>
      )}

      <section style={{ marginTop: '2rem' }}>
        <h2>How to call</h2>
        <pre className="code-block">{`curl -s http://127.0.0.1:20129/v1/chat/completions \\
  -H "Authorization: Bearer <member-key>" \\
  -H "Content-Type: application/json" \\
  -d '{
    "model": "glm/glm-5.2",
    "messages": [{"role":"user","content":"hi"}]
  }'

# List models (OpenAI shape)
curl -s http://127.0.0.1:20129/v1/models \\
  -H "Authorization: Bearer <member-key>"`}</pre>
      </section>
    </div>
  );
}
