"use client";
import { useEffect, useMemo, useState } from "react";
import { sortMemberRows, groupMemberRows } from "./membersTable.js";

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

function lastUsedCell(v) {
  if (!v) return "—";
  const d = new Date(v);
  return isNaN(d.getTime()) ? "—" : <span suppressHydrationWarning>{d.toLocaleString()}</span>;
}

export default function MembersTab({ period }) {
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [sortBy, setSortBy] = useState("cost");
  const [sortDir, setSortDir] = useState("desc");
  const [expanded, setExpanded] = useState(() => new Set());

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

  const groups = useMemo(() => groupMemberRows(rows), [rows]);
  const sortedGroups = useMemo(() => {
    const order = sortMemberRows(groups.map((g) => g.summary), sortBy, sortDir);
    return order.map((s) => groups.find((g) => g.summary === s));
  }, [groups, sortBy, sortDir]);

  function toggle(k) {
    if (k === sortBy) setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    else { setSortBy(k); setSortDir("desc"); }
  }

  function toggleExpand(key) {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }

  if (loading) return <div className="text-muted">Loading…</div>;
  if (error) return <div className="text-red-500">Failed to load: {error}</div>;
  if (!sortedGroups.length) return <div className="text-muted">No member usage in this period.</div>;

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
          {sortedGroups.flatMap((g) => {
            const key = g.summary.id || g.summary.keyName;
            const isOpen = expanded.has(key);
            const out = [
              <tr
                key={`g-${key}`}
                onClick={() => toggleExpand(key)}
                className="cursor-pointer border-t border-border bg-muted/30 hover:bg-muted/50"
              >
                <td className="px-2 py-1 font-mono">{isOpen ? "▼ " : "▶ "}{g.summary.keyName}</td>
                <td className="px-2 py-1 font-mono text-muted">{g.cells.length} model{g.cells.length === 1 ? "" : "s"}</td>
                <td className="px-2 py-1 font-mono">{fmt(g.summary.requests)}</td>
                <td className="px-2 py-1 font-mono">{fmt(g.summary.promptTokens)}</td>
                <td className="px-2 py-1 font-mono">{fmt(g.summary.completionTokens)}</td>
                <td className="px-2 py-1 font-mono">—</td>
                <td className="px-2 py-1 font-mono">—</td>
                <td className="px-2 py-1 font-mono">—</td>
                <td className="px-2 py-1 font-mono">—</td>
                <td className="px-2 py-1 font-mono">{fmt(g.summary.cost)}</td>
                <td className="px-2 py-1 font-mono">{lastUsedCell(g.summary.lastUsed)}</td>
              </tr>,
            ];
            if (isOpen) {
              for (const r of g.cells) {
                out.push(
                  <tr key={`c-${key}-${r.model}-${r.provider}`} className="border-t border-border bg-muted/10">
                    <td className="px-2 py-1 font-mono"></td>
                    <td className="px-2 py-1 pl-6 font-mono">{r.model}</td>
                    <td className="px-2 py-1 font-mono">{fmt(r.requests)}</td>
                    <td className="px-2 py-1 font-mono">{fmt(r.promptTokens)}</td>
                    <td className="px-2 py-1 font-mono">{fmt(r.completionTokens)}</td>
                    <td className="px-2 py-1 font-mono">{fmt(r.meanTPS)}</td>
                    <td className="px-2 py-1 font-mono">{fmt(r.p50TPS)}</td>
                    <td className="px-2 py-1 font-mono">{fmt(r.p95TPS)}</td>
                    <td className="px-2 py-1 font-mono">{fmt(r.throughputTPS)}</td>
                    <td className="px-2 py-1 font-mono">{fmt(r.cost)}</td>
                    <td className="px-2 py-1 font-mono">{lastUsedCell(r.lastUsed)}</td>
                  </tr>
                );
              }
            }
            return out;
          })}
        </tbody>
      </table>
      <p className="mt-2 text-xs text-muted">
        TPS = output tokens / generation seconds. Cells with no latency data show —. Throughput is weighted by generation time, not the mean of per-request TPS. Summary rows show totals; expand a member for per-model TPS.
      </p>
    </div>
  );
}
