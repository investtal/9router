import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { fetchRequests, type RequestLogRow } from '../lib/api';

export const Route = createFileRoute('/usage')({
  component: UsagePage,
});

function UsagePage() {
  const [items, setItems] = useState<RequestLogRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [model, setModel] = useState('');

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    fetchRequests({
      limit: 50,
      model: model.trim() || undefined,
    })
      .then((res) => {
        if (!cancelled) {
          setItems(res.items);
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
  }, [model]);

  return (
    <div>
      <h1>Usage / requests</h1>
      <div className="row">
        <label>
          Model filter{' '}
          <input
            value={model}
            placeholder="optional"
            onChange={(e) => setModel(e.target.value)}
          />
        </label>
      </div>
      {error ? <div className="error card">{error}</div> : null}
      {loading ? <p className="muted">Loading…</p> : null}
      <div className="card" style={{ overflowX: 'auto' }}>
        <table>
          <thead>
            <tr>
              <th>Time</th>
              <th>Model</th>
              <th>Status</th>
              <th>Member</th>
              <th>Tokens</th>
              <th>Latency</th>
            </tr>
          </thead>
          <tbody>
            {items.length === 0 && !loading ? (
              <tr>
                <td colSpan={6} className="muted">
                  No requests
                </td>
              </tr>
            ) : null}
            {items.map((row) => (
              <tr key={row.id}>
                <td className="mono">{row.created_at}</td>
                <td>{row.model ?? '—'}</td>
                <td>{row.status ?? '—'}</td>
                <td className="mono">{row.member_key_id ?? '—'}</td>
                <td>
                  {row.prompt_tokens}/{row.completion_tokens}
                  {row.usage_incomplete ? ' *' : ''}
                </td>
                <td>
                  {row.latency_ms != null ? `${row.latency_ms}ms` : '—'}
                  {row.ttft_ms != null ? ` / ttft ${row.ttft_ms}ms` : ''}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
