import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { fetchMembers, RANGES, type MemberModelCell, type Range } from '../lib/api';
import { formatCost, formatNumber, providerFromModel } from '../lib/format';
import { ProviderLogo } from '../lib/providerLogo';
import { RankedInputTokens, RankedOutputTokens } from '../lib/RankValue';

export const Route = createFileRoute('/members')({
  component: MembersPage,
});

function MembersPage() {
  const [range, setRange] = useState<Range>('7d');
  const [rows, setRows] = useState<MemberModelCell[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    fetchMembers(range)
      .then((data) => {
        if (!cancelled) {
          setRows(data);
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
      <h1>Members</h1>
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
      {loading ? <p className="muted">Loading…</p> : null}
      <div className="card" style={{ overflowX: 'auto' }}>
        <table>
          <thead>
            <tr>
              <th>Member</th>
              <th>Model</th>
              <th>Requests</th>
              <th>Input</th>
              <th>Output</th>
              <th>Cost est.</th>
            </tr>
          </thead>
          <tbody>
            {rows.length === 0 && !loading ? (
              <tr>
                <td colSpan={6} className="muted">
                  No member usage
                </td>
              </tr>
            ) : null}
            {rows.map((row, i) => {
              const prov = providerFromModel(row.model);
              return (
                <tr key={`${row.member_key_id}-${row.model}-${i}`}>
                  <td>
                    {row.member_name ?? '—'}
                    <div className="muted mono" style={{ fontSize: '0.8em' }}>
                      {row.member_key_id.slice(0, 8)}…
                    </div>
                  </td>
                  <td>
                    <span className="provider-chip">
                      <ProviderLogo provider={prov} size={18} />
                      <span>{row.model}</span>
                    </span>
                  </td>
                  <td>{formatNumber(row.request_count)}</td>
                  <td>
                    <RankedInputTokens n={row.prompt_tokens} />
                  </td>
                  <td>
                    <RankedOutputTokens n={row.completion_tokens} />
                  </td>
                  <td>{formatCost(row.cost_est)}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
    </div>
  );
}
