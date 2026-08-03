import { createFileRoute } from '@tanstack/react-router';
import { useEffect, useState } from 'react';
import { fetchMembers, RANGES, type MemberModelCell, type Range } from '../lib/api';

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
              <th>Prompt</th>
              <th>Completion</th>
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
            {rows.map((row, i) => (
              <tr key={`${row.member_key_id}-${row.model}-${i}`}>
                <td>
                  {row.member_name ?? '—'}
                  <div className="muted mono">{row.member_key_id}</div>
                </td>
                <td>{row.model}</td>
                <td>{row.request_count}</td>
                <td>{row.prompt_tokens}</td>
                <td>{row.completion_tokens}</td>
                <td>{row.cost_est.toFixed(4)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
