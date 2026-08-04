import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import {
  fetchOverview,
  RANGES,
  type Range,
  type UsageOverview,
} from '../lib/api';
import { formatCost, formatDateTime, formatNumber } from '../lib/format';

export const Route = createFileRoute('/')({
  component: OverviewPage,
});

function OverviewPage() {
  const [range, setRange] = useState<Range>('7d');
  const [data, setData] = useState<UsageOverview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    fetchOverview(range)
      .then((o) => {
        if (!cancelled) {
          setData(o);
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
  }, [range]);

  return (
    <div>
      <h1>Overview</h1>
      <div className="row">
        <label>
          Range{' '}
          <select value={range} onChange={(e) => setRange(e.target.value as Range)}>
            {RANGES.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
        </label>
      </div>
      {error ? <div className="error card">{error}</div> : null}
      {loading && !data ? <p className="muted">Loading…</p> : null}
      {data ? (
        <div className="grid">
          <Stat label="Requests" value={formatNumber(data.request_count)} />
          <Stat label="Input tokens" value={formatNumber(data.prompt_tokens)} />
          <Stat label="Output tokens" value={formatNumber(data.completion_tokens)} />
          <Stat
            label="Total tokens"
            value={formatNumber(data.prompt_tokens + data.completion_tokens)}
          />
          <Stat label="Cached tokens" value={formatNumber(data.cached_tokens)} />
          <Stat label="Cost est." value={formatCost(data.cost_est)} />
        </div>
      ) : null}
      {data ? (
        <p className="muted" style={{ marginTop: '1rem' }}>
          Window {formatDateTime(data.from)} → {formatDateTime(data.to)}
        </p>
      ) : null}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string | number }) {
  return (
    <div className="stat">
      <div className="label">{label}</div>
      <div className="value">{value}</div>
    </div>
  );
}
