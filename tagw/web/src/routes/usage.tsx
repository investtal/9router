import { createFileRoute } from '@tanstack/react-router';
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import {
  fetchRequestDetail,
  fetchRequests,
  type RequestLogRow,
} from '../lib/api';

export const Route = createFileRoute('/usage')({
  component: UsagePage,
});

function UsagePage() {
  const [items, setItems] = useState<RequestLogRow[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const [model, setModel] = useState('');
  const [selected, setSelected] = useState<RequestLogRow | null>(null);
  const [detailLoading, setDetailLoading] = useState(false);

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

  const openDetail = useCallback(async (row: RequestLogRow) => {
    setSelected(row);
    setDetailLoading(true);
    try {
      const full = await fetchRequestDetail(row.id);
      setSelected(full);
    } catch {
      // List row already has the same fields; keep it open.
    } finally {
      setDetailLoading(false);
    }
  }, []);

  useEffect(() => {
    if (!selected) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setSelected(null);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [selected]);

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
      <p className="muted" style={{ marginTop: 0 }}>
        Click a row to open request detail (metadata + tokens + latency). Full message bodies are not
        stored in tagw yet (9router-style payload capture is a later enhancement).
      </p>
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
              <tr
                key={row.id}
                className={`clickable-row${selected?.id === row.id ? ' selected' : ''}`}
                onClick={() => void openDetail(row)}
                onKeyDown={(e) => {
                  if (e.key === 'Enter' || e.key === ' ') {
                    e.preventDefault();
                    void openDetail(row);
                  }
                }}
                tabIndex={0}
                role="button"
                title="View request detail"
              >
                <td className="mono">{row.created_at}</td>
                <td>{row.model ?? '—'}</td>
                <td>{row.status ?? '—'}</td>
                <td className="mono">{row.member_key_id?.slice(0, 8) ?? '—'}</td>
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

      {selected ? (
        <RequestDetailModal
          detail={selected}
          loading={detailLoading}
          onClose={() => setSelected(null)}
        />
      ) : null}
    </div>
  );
}

function RequestDetailModal({
  detail,
  loading,
  onClose,
}: {
  detail: RequestLogRow;
  loading: boolean;
  onClose: () => void;
}) {
  return (
    <div
      className="modal-backdrop"
      role="presentation"
      onClick={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="modal-panel" role="dialog" aria-labelledby="req-detail-title">
        <div className="modal-header">
          <h2 id="req-detail-title">Request detail</h2>
          <button type="button" className="secondary" onClick={onClose}>
            Close
          </button>
        </div>
        {loading ? <p className="muted">Refreshing…</p> : null}

        <Section title="Summary">
          <dl className="detail-grid">
            <dt>ID</dt>
            <dd className="mono">{detail.id}</dd>
            <dt>Time</dt>
            <dd className="mono">{detail.created_at}</dd>
            <dt>Model</dt>
            <dd>{detail.model ?? '—'}</dd>
            <dt>Status</dt>
            <dd>{detail.status ?? '—'}</dd>
            <dt>Tool / client</dt>
            <dd>{detail.tool ?? '—'}</dd>
            <dt>Member key</dt>
            <dd className="mono">{detail.member_key_id ?? '—'}</dd>
            <dt>Provider</dt>
            <dd className="mono">{detail.provider_id ?? '—'}</dd>
            <dt>Account</dt>
            <dd className="mono">{detail.account_id ?? '—'}</dd>
          </dl>
        </Section>

        <Section title="Tokens & cost">
          <dl className="detail-grid">
            <dt>Input</dt>
            <dd>{detail.prompt_tokens.toLocaleString()}</dd>
            <dt>Cached</dt>
            <dd>{detail.cached_tokens.toLocaleString()}</dd>
            <dt>Output</dt>
            <dd>{detail.completion_tokens.toLocaleString()}</dd>
            <dt>Est. cost</dt>
            <dd>${detail.cost_est.toFixed(6)}</dd>
            <dt>Usage incomplete</dt>
            <dd>{detail.usage_incomplete ? 'yes' : 'no'}</dd>
          </dl>
        </Section>

        <Section title="Latency">
          <dl className="detail-grid">
            <dt>TTFT</dt>
            <dd>{detail.ttft_ms != null ? `${detail.ttft_ms} ms` : '—'}</dd>
            <dt>Total</dt>
            <dd>{detail.latency_ms != null ? `${detail.latency_ms} ms` : '—'}</dd>
          </dl>
        </Section>

        {detail.error ? (
          <Section title="Error">
            <pre className="detail-pre">{detail.error}</pre>
          </Section>
        ) : null}

        <Section title="Messages / body" defaultOpen={false}>
          <p className="muted" style={{ margin: 0 }}>
            tagw currently logs request <strong>metadata and token usage</strong> only. Full prompt /
            response payloads (like 9router request details) are not persisted yet. Use live logs for
            recent stream events, or open a follow-up if you need body capture.
          </p>
        </Section>
      </div>
    </div>
  );
}

function Section({
  title,
  children,
  defaultOpen = true,
}: {
  title: string;
  children: ReactNode;
  defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="detail-section">
      <button type="button" className="detail-section-toggle" onClick={() => setOpen((v) => !v)}>
        <span>{open ? '▼' : '▶'}</span> {title}
      </button>
      {open ? <div className="detail-section-body">{children}</div> : null}
    </div>
  );
}
