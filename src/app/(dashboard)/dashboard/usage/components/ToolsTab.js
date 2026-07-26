"use client";

import { useEffect, useMemo, useState } from "react";
import Card from "@/shared/components/Card";
import Button from "@/shared/components/Button";
import { formatBytesish } from "@/shared/utils/requestDetailParse";
import { cn } from "@/shared/utils/cn";

const COLS = [
  { key: "name", label: "Tool", align: "left" },
  { key: "calls", label: "Calls", align: "right" },
  { key: "results", label: "Results", align: "right" },
  { key: "requestCount", label: "Requests", align: "right" },
  { key: "chars", label: "Payload", align: "right" },
  { key: "avgCharsPerCall", label: "Avg / call", align: "right" },
  { key: "declaredCount", label: "Declared in", align: "right" },
];

function sortRows(rows, sortBy, sortDir) {
  const mul = sortDir === "asc" ? 1 : -1;
  return [...rows].sort((a, b) => {
    let va = a[sortBy];
    let vb = b[sortBy];
    if (typeof va === "string") va = va.toLowerCase();
    if (typeof vb === "string") vb = vb.toLowerCase();
    if (va == null && vb == null) return 0;
    if (va == null) return 1;
    if (vb == null) return -1;
    if (va < vb) return -1 * mul;
    if (va > vb) return 1 * mul;
    return a.name.localeCompare(b.name);
  });
}

function exportCsv(tools, period) {
  const headers = ["tool", "calls", "results", "requests", "payload_chars", "avg_chars_per_call", "declared_in_requests"];
  const lines = [headers.join(",")];
  for (const t of tools) {
    lines.push([
      JSON.stringify(t.name),
      t.calls || 0,
      t.results || 0,
      t.requestCount || 0,
      t.chars || 0,
      t.avgCharsPerCall || 0,
      t.declaredCount || 0,
    ].join(","));
  }
  const blob = new Blob([lines.join("\n")], { type: "text/csv;charset=utf-8" });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = `9router-tools-${period}-${new Date().toISOString().slice(0, 10)}.csv`;
  a.click();
  URL.revokeObjectURL(url);
}

export default function ToolsTab({ period }) {
  const [tools, setTools] = useState([]);
  const [meta, setMeta] = useState({ scanned: 0, withActivity: 0, limit: 0 });
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const [sortBy, setSortBy] = useState("chars");
  const [sortDir, setSortDir] = useState("desc");
  const [hideDeclaredOnly, setHideDeclaredOnly] = useState(true);
  const [query, setQuery] = useState("");

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);
    fetch(`/api/usage/tools?period=${encodeURIComponent(period)}`)
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .then((d) => {
        if (cancelled) return;
        setTools(d.tools || []);
        setMeta({ scanned: d.scanned || 0, withActivity: d.withActivity || 0, limit: d.limit || 0 });
      })
      .catch((e) => {
        if (!cancelled) setError(e.message);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => { cancelled = true; };
  }, [period]);

  const filtered = useMemo(() => {
    let rows = tools;
    if (hideDeclaredOnly) rows = rows.filter((t) => !t.declaredOnly);
    if (query.trim()) {
      const q = query.trim().toLowerCase();
      rows = rows.filter((t) => t.name.toLowerCase().includes(q));
    }
    return sortRows(rows, sortBy, sortDir);
  }, [tools, hideDeclaredOnly, query, sortBy, sortDir]);

  const totals = useMemo(() => {
    return filtered.reduce(
      (acc, t) => {
        acc.calls += t.calls || 0;
        acc.results += t.results || 0;
        acc.chars += t.chars || 0;
        return acc;
      },
      { calls: 0, results: 0, chars: 0 }
    );
  }, [filtered]);

  function toggleSort(key) {
    if (key === sortBy) setSortDir((d) => (d === "asc" ? "desc" : "asc"));
    else {
      setSortBy(key);
      setSortDir(key === "name" ? "asc" : "desc");
    }
  }

  if (loading) {
    return (
      <div className="flex items-center gap-2 text-text-muted py-8 justify-center">
        <span className="material-symbols-outlined animate-spin text-[20px]">progress_activity</span>
        Loading tool stats…
      </div>
    );
  }

  if (error) {
    return <div className="text-red-500 text-sm">Failed to load tool stats: {error}</div>;
  }

  return (
    <div className="flex min-w-0 flex-col gap-4">
      <Card padding="md">
        <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <div className="text-sm text-text-muted">
            Scanned <span className="font-mono text-text-main">{meta.scanned}</span> stored requests
            {" · "}
            <span className="font-mono text-text-main">{meta.withActivity}</span> with tool activity
            {meta.limit ? <> (cap {meta.limit})</> : null}
            <p className="text-xs mt-1">
              Payload is character count of tool args + results (proxy for context share). Not a separate provider quota.
              Requires Observability ON.
            </p>
          </div>
          <div className="flex flex-wrap items-center gap-2">
            <input
              type="search"
              placeholder="Filter tool…"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              className="h-9 px-3 rounded-lg border border-border bg-surface text-sm text-text-main min-w-[140px]"
            />
            <label className="flex items-center gap-1.5 text-xs text-text-muted cursor-pointer select-none">
              <input
                type="checkbox"
                checked={hideDeclaredOnly}
                onChange={(e) => setHideDeclaredOnly(e.target.checked)}
              />
              Hide declared-only
            </label>
            <Button
              variant="outline"
              size="sm"
              onClick={() => exportCsv(filtered, period)}
              disabled={!filtered.length}
            >
              Export CSV
            </Button>
          </div>
        </div>

        <div className="grid grid-cols-3 gap-2 mt-4 text-xs">
          <div className="rounded-md border border-border px-3 py-2">
            <div className="text-text-muted uppercase tracking-wide text-[10px]">Total calls</div>
            <div className="font-mono text-sm">{totals.calls.toLocaleString()}</div>
          </div>
          <div className="rounded-md border border-border px-3 py-2">
            <div className="text-text-muted uppercase tracking-wide text-[10px]">Total results</div>
            <div className="font-mono text-sm">{totals.results.toLocaleString()}</div>
          </div>
          <div className="rounded-md border border-border px-3 py-2">
            <div className="text-text-muted uppercase tracking-wide text-[10px]">Total payload</div>
            <div className="font-mono text-sm">{formatBytesish(totals.chars)}</div>
          </div>
        </div>
      </Card>

      <Card padding="none">
        {!filtered.length ? (
          <div className="p-8 text-center text-text-muted text-sm">
            No tool activity in this period. Make requests with Observability enabled, or uncheck “Hide declared-only”.
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full min-w-[720px] text-sm">
              <thead>
                <tr className="border-b border-border">
                  {COLS.map((c) => (
                    <th
                      key={c.key}
                      className={cn(
                        "px-4 py-3 font-semibold text-text-muted cursor-pointer select-none whitespace-nowrap",
                        c.align === "right" ? "text-right" : "text-left"
                      )}
                      onClick={() => toggleSort(c.key)}
                    >
                      {c.label}
                      {sortBy === c.key ? (sortDir === "asc" ? " ↑" : " ↓") : ""}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody className="divide-y divide-border/60">
                {filtered.map((t) => (
                  <tr key={t.name} className="hover:bg-bg-subtle/60">
                    <td className="px-4 py-2.5 font-mono text-xs">
                      {t.name}
                      {t.declaredOnly && (
                        <span className="ml-2 text-[10px] text-text-muted normal-case font-sans">declared only</span>
                      )}
                    </td>
                    <td className="px-4 py-2.5 text-right font-mono">{(t.calls || 0).toLocaleString()}</td>
                    <td className="px-4 py-2.5 text-right font-mono">{(t.results || 0).toLocaleString()}</td>
                    <td className="px-4 py-2.5 text-right font-mono">{(t.requestCount || 0).toLocaleString()}</td>
                    <td className="px-4 py-2.5 text-right font-mono" title={`${t.chars || 0} chars`}>
                      {formatBytesish(t.chars || 0)}
                    </td>
                    <td className="px-4 py-2.5 text-right font-mono">
                      {t.avgCharsPerCall ? formatBytesish(t.avgCharsPerCall) : "—"}
                    </td>
                    <td className="px-4 py-2.5 text-right font-mono text-text-muted">
                      {(t.declaredCount || 0).toLocaleString()}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Card>
    </div>
  );
}
