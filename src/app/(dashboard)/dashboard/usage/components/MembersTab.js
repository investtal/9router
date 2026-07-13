"use client";
import { useEffect, useMemo, useState } from "react";
import { sortMemberRows } from "./membersTable.js";

const COLS = [
  { key: "keyName", label: "Member" },
  { key: "model", label: "Model" },
  { key: "requests", label: "Requests" },
  { key: "promptTokens", label: "Input" },
  { key: "completionTokens", label: "Output" },
  { key: "meanTPS", label: "Mean TPS" },
  { key: "p50TPS", label: "p50 TPS" },
  { key: "p95TPS", label: "p95 TPS" },
  { key: "throughputTPS", label: "Throughput" },
  { key: "cost", label: "Cost" },
  { key: "lastUsed", label: "Last used" },
];

function fmt(n) {
  if (n === null || n === undefined) return "—";
  if (typeof n === "number") return n.toLocaleString(undefined, { maximumFractionDigits: 1 });
  return n;
}

export default function MembersTab({ period }) {
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [sortBy, setSortBy] = useState("cost");
  const [sortDir, setSortDir] = useState("desc");

  useEffect(() => {
    let cancelled = false;
    setLoading(true); setError(null);
    fetch(`/api/usage/members?period=${encodeURIComponent(period)}`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d) => { if (!cancelled) setRows(d.members || []); })
      .catch((e) => { if (!cancelled) setError(e.message); })
      .finally(() => { if (!cancelled) setLoading(false); });
    return () => { cancelled = true; };
  }, [period]);

  const sorted = useMemo(() => sortMemberRows(rows, sortBy, sortDir), [rows, sortBy, sortDir]);

  function toggle(k) {
    if (k === sortBy) setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    else { setSortBy(k); setSortDir("desc"); }
  }

  if (loading) return <div className="text-muted">Loading…</div>;
  if (error) return <div className="text-red-500">Failed to load: {error}</div>;
  if (!sorted.length) return <div className="text-muted">No member usage in this period.</div>;

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-sm">
        <thead>
          <tr>
            {COLS.map((c) => (
              <th
                key={c.key}
                onClick={() => toggle(c.key)}
                className="cursor-pointer px-2 py-1 text-left select-none"
              >
                {c.label}{sortBy === c.key ? (sortDir === "asc" ? " ▲" : " ▼") : ""}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {sorted.map((r, i) => (
            <tr key={`${r.id || r.keyName}-${r.model}-${i}`} className="border-t border-border">
              {COLS.map((c) => (
                <td key={c.key} className="px-2 py-1 font-mono">
                  {c.key === "lastUsed"
                    ? r.lastUsed ? new Date(r.lastUsed).toLocaleString() : "—"
                    : fmt(r[c.key])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <p className="mt-2 text-xs text-muted">
        TPS = output tokens / generation seconds. Cells with no latency data show —. Throughput is weighted by generation time, not the mean of per-request TPS.
      </p>
    </div>
  );
}
