import { createFileRoute } from '@tanstack/react-router';
import { useCallback, useEffect, useState, type ReactNode } from 'react';
import {
  fetchRequestDetail,
  fetchRequests,
  type RequestLogRow,
} from '../lib/api';
import { formatCost, formatDateTime, formatNumber, providerFromModel } from '../lib/format';
import { ProviderChip, ProviderLogo } from '../lib/providerLogo';
import {
  RankedInputTokens,
  RankedLatency,
  RankedOutputTokens,
  RankedTtft,
  RankLegend,
  TokensInOut,
} from '../lib/RankValue';

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
      // Always re-fetch detail — list rows omit full bodies by design.
      const full = await fetchRequestDetail(row.id);
      setSelected(full);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
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
      <RankLegend />
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
              <th>Body</th>
              <th>Tokens (in/out)</th>
              <th>Latency</th>
            </tr>
          </thead>
          <tbody>
            {items.length === 0 && !loading ? (
              <tr>
                <td colSpan={7} className="muted">
                  No requests
                </td>
              </tr>
            ) : null}
            {items.map((row) => {
              const prov = providerFromModel(row.model);
              const hasBody = Boolean(row.has_request_body || row.has_response_body);
              return (
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
                  <td className="time-cell">{formatDateTime(row.created_at)}</td>
                  <td>
                    <span className="provider-chip">
                      <ProviderLogo provider={prov} size={18} />
                      <span>{row.model ?? '—'}</span>
                    </span>
                  </td>
                  <td>{row.status ?? '—'}</td>
                  <td title={row.member_key_id ?? undefined}>
                    {row.member_name || row.member_key_id?.slice(0, 8) || '—'}
                  </td>
                  <td>
                    {hasBody ? (
                      <span className="badge on" title="Request/response body captured">
                        captured
                      </span>
                    ) : (
                      <span className="badge" title="No body (pre-capture or empty)">
                        —
                      </span>
                    )}
                  </td>
                  <td>
                    <TokensInOut
                      input={row.prompt_tokens}
                      output={row.completion_tokens}
                      incomplete={row.usage_incomplete}
                    />
                  </td>
                  <td>
                    <span className="tokens-split">
                      <RankedLatency ms={row.latency_ms} />
                      {row.ttft_ms != null ? (
                        <>
                          <span className="sep">/</span>
                          <RankedTtft ms={row.ttft_ms} />
                        </>
                      ) : null}
                    </span>
                  </td>
                </tr>
              );
            })}
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

function prettyJson(raw: string | null | undefined): string {
  if (!raw) return '';
  try {
    return JSON.stringify(JSON.parse(raw), null, 2);
  } catch {
    return raw;
  }
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
  const requestPretty = prettyJson(detail.request_body);
  const responsePretty = prettyJson(detail.response_body);
  const prov = providerFromModel(detail.model);

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
            <dd className="time-cell">{formatDateTime(detail.created_at)}</dd>
            <dt>Model</dt>
            <dd>
              <ProviderChip provider={prov} label={detail.model ?? '—'} />
            </dd>
            <dt>Status</dt>
            <dd>{detail.status ?? '—'}</dd>
            <dt>Tool / client</dt>
            <dd>{detail.tool ?? '—'}</dd>
            <dt>Member</dt>
            <dd>
              {detail.member_name || '—'}
              {detail.member_key_id ? (
                <span className="muted mono" style={{ marginLeft: 8, fontSize: '0.85em' }}>
                  ({detail.member_key_id.slice(0, 8)}…)
                </span>
              ) : null}
            </dd>
            <dt>Provider</dt>
            <dd className="mono">{detail.provider_id ?? '—'}</dd>
            <dt>Account</dt>
            <dd className="mono">{detail.account_id ?? '—'}</dd>
          </dl>
        </Section>

        <Section title="Tokens & cost">
          <dl className="detail-grid">
            <dt>Input</dt>
            <dd>
              <RankedInputTokens n={detail.prompt_tokens} />
            </dd>
            <dt>Cached</dt>
            <dd>{formatNumber(detail.cached_tokens)}</dd>
            <dt>Output</dt>
            <dd>
              <RankedOutputTokens n={detail.completion_tokens} />
            </dd>
            <dt>Est. cost</dt>
            <dd>{formatCost(detail.cost_est)}</dd>
            <dt>Usage incomplete</dt>
            <dd>{detail.usage_incomplete ? 'yes' : 'no'}</dd>
          </dl>
        </Section>

        <Section title="Latency">
          <dl className="detail-grid">
            <dt>TTFT</dt>
            <dd>
              <RankedTtft ms={detail.ttft_ms} />
            </dd>
            <dt>Total</dt>
            <dd>
              <RankedLatency ms={detail.latency_ms} />
            </dd>
          </dl>
        </Section>

        {detail.error ? (
          <Section title="Error">
            <pre className="detail-pre">{detail.error}</pre>
          </Section>
        ) : null}

        <Section title="Request body" defaultOpen>
          {requestPretty ? (
            <pre className="detail-pre">{requestPretty}</pre>
          ) : (
            <p className="muted" style={{ margin: 0 }}>
              {detail.has_request_body === false || !detail.request_body
                ? 'No request body on this row. Capture is ON by default for new traffic — open a row marked “captured” in the Body column (older rows from before capture stay empty).'
                : 'Loading body…'}
            </p>
          )}
        </Section>

        <Section title="Response body" defaultOpen={Boolean(responsePretty)}>
          {responsePretty ? (
            <pre className="detail-pre">{responsePretty}</pre>
          ) : (
            <p className="muted" style={{ margin: 0 }}>
              No response body on this row (legacy request, or stream ended with empty payload).
            </p>
          )}
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
