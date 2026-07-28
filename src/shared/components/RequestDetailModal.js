"use client";

import { useEffect, useMemo, useState, useCallback } from "react";
import Modal from "./Modal";
import Button from "./Button";
import { cn } from "@/shared/utils/cn";
import {
  extractSystemPrompt,
  extractMessages,
  extractDeclaredTools,
  extractToolActivity,
  formatBytesish,
} from "@/shared/utils/requestDetailParse";
import {
  buildSingleRequestToon,
  downloadTextFile,
  safeFilenamePart,
} from "@/shared/utils/toonExport";

const EMPTY_OBJ = Object.freeze({});

function Section({ title, children, defaultOpen = true, badge = null }) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="border border-border rounded-lg overflow-hidden">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="w-full flex items-center justify-between gap-2 px-3 py-2 bg-bg-subtle hover:bg-bg-hover transition-colors text-left"
      >
        <span className="text-xs font-semibold text-text-main uppercase tracking-wide flex items-center gap-2">
          {title}
          {badge != null && (
            <span className="normal-case font-mono text-[10px] px-1.5 py-0.5 rounded bg-black/5 dark:bg-white/10 text-text-muted">
              {badge}
            </span>
          )}
        </span>
        <span className={cn("material-symbols-outlined text-[18px] text-text-muted transition-transform", open && "rotate-90")}>
          chevron_right
        </span>
      </button>
      {open && <div className="p-3 border-t border-border">{children}</div>}
    </div>
  );
}

function ScrollPre({ children, className }) {
  return (
    <pre
      className={cn(
        "max-h-[min(50vh,420px)] overflow-auto rounded-md border border-border bg-black/[0.03] dark:bg-white/[0.04]",
        "p-3 font-mono text-[11px] leading-relaxed text-text-main whitespace-pre-wrap break-words",
        className
      )}
    >
      {children}
    </pre>
  );
}

function roleTone(role) {
  const r = String(role || "").toLowerCase();
  if (r === "user" || r === "human") return "bg-sky-500/15 text-sky-700 dark:text-sky-300";
  if (r === "assistant" || r === "model" || r === "ai") return "bg-violet-500/15 text-violet-700 dark:text-violet-300";
  if (r === "system" || r === "developer") return "bg-amber-500/15 text-amber-800 dark:text-amber-200";
  if (r === "tool" || r === "function") return "bg-emerald-500/15 text-emerald-700 dark:text-emerald-300";
  return "bg-black/5 dark:bg-white/10 text-text-muted";
}

function MessageRow({ message, defaultOpen = false }) {
  const [open, setOpen] = useState(defaultOpen);
  const chars = message.content?.length || 0;
  const empty = !chars;
  const preview = message.preview || (empty ? "(empty)" : message.content.replace(/\s+/g, " ").trim().slice(0, 200));

  return (
    <div className="border-t border-border/60 first:border-t-0">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        className="w-full flex items-center gap-2 px-1.5 py-1.5 text-left hover:bg-bg-hover/50 transition-colors"
      >
        <span className={cn("material-symbols-outlined text-[14px] text-text-muted shrink-0 transition-transform", open && "rotate-90")}>
          chevron_right
        </span>
        <span className="font-mono text-[10px] text-text-muted shrink-0 w-5">#{message.index}</span>
        <span className={cn("text-[10px] font-semibold uppercase tracking-wide px-1.5 py-0.5 rounded shrink-0", roleTone(message.role))}>
          {message.role || "unknown"}
        </span>
        <span className={cn("flex-1 min-w-0 text-[11px] truncate", empty ? "text-text-muted italic" : "text-text-main")}>
          {preview}
        </span>
        <span className="font-mono text-[10px] text-text-muted shrink-0">{formatBytesish(chars)}</span>
      </button>
      {open && (
        <div className="px-1.5 pb-2 pt-0.5">
          {empty ? (
            <p className="text-xs text-text-muted italic">No text content on this turn (payload may live only on tool fields).</p>
          ) : (
            <ScrollPre className="max-h-80 border-border/70">{message.content}</ScrollPre>
          )}
        </div>
      )}
    </div>
  );
}

const MSG_INITIAL_LIMIT = 40;

function MessagesList({ messages }) {
  const [filter, setFilter] = useState("all"); // all | chat | tools | nonEmpty
  const [query, setQuery] = useState("");
  const [limit, setLimit] = useState(MSG_INITIAL_LIMIT);

  useEffect(() => {
    setLimit(MSG_INITIAL_LIMIT);
  }, [filter, query]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const matches = (m) => {
      if (q && !(m.content || "").toLowerCase().includes(q) && !(m.role || "").toLowerCase().includes(q)) {
        return false;
      }
      if (filter === "chat") {
        const r = String(m.role || "").toLowerCase();
        return r === "user" || r === "assistant" || r === "human" || r === "model";
      }
      if (filter === "tools") {
        const r = String(m.role || "").toLowerCase();
        return r === "tool" || r === "function" || (m.content || "").includes("[tool_");
      }
      if (filter === "nonEmpty") {
        return (m.content || "").trim().length > 0;
      }
      return true;
    };
    return messages.filter(matches);
  }, [messages, filter, query]);

  const openFirst = useMemo(() => {
    const idx = filtered.findIndex((m) => (m.content || "").trim().length > 0);
    return idx >= 0 ? filtered[idx].index : -1;
  }, [filtered]);

  if (!messages.length) {
    return <p className="text-sm text-text-muted">No messages array captured.</p>;
  }

  const shown = filtered.slice(0, limit);
  const hidden = filtered.length - shown.length;
  const autoExpand = filtered.length <= 12;

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-1.5">
        {[
          ["all", `All (${messages.length})`],
          ["chat", "User / assistant"],
          ["tools", "Tool turns"],
          ["nonEmpty", "With text"],
        ].map(([key, label]) => (
          <button
            key={key}
            type="button"
            onClick={() => setFilter(key)}
            className={cn(
              "px-2 py-0.5 rounded text-[11px] font-medium",
              filter === key ? "bg-primary text-white" : "bg-bg-subtle text-text-muted hover:bg-bg-hover"
            )}
          >
            {label}
          </button>
        ))}
        <input
          type="search"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Search messages..."
          className="ml-auto h-7 w-40 rounded border border-black/10 dark:border-white/10 bg-surface px-2 text-[11px] text-text-main focus:outline-none focus:ring-2 focus:ring-primary/20"
        />
      </div>
      {filtered.length === 0 ? (
        <p className="text-sm text-text-muted py-2">No messages match this filter.</p>
      ) : (
        <>
          <span className="text-[10px] text-text-muted">
            Showing {shown.length} of {filtered.length} · click a row to expand
          </span>
          <div className="rounded-md border border-border overflow-hidden bg-surface">
            {shown.map((m) => (
              <MessageRow key={m.index} message={m} defaultOpen={autoExpand && m.index === openFirst} />
            ))}
          </div>
          {hidden > 0 && (
            <button
              type="button"
              onClick={() => setLimit((l) => l + MSG_INITIAL_LIMIT)}
              className="self-start text-[11px] font-medium text-primary hover:underline"
            >
              Show {Math.min(MSG_INITIAL_LIMIT, hidden)} more ({hidden} hidden)
            </button>
          )}
        </>
      )}
    </div>
  );
}

function fmtTokens(n) {
  if (n == null) return "—";
  return Number(n).toLocaleString();
}

/**
 * Modal that loads / renders full request observability detail.
 * Pass either `detail` (already loaded) or `detailId` to fetch.
 */
export default function RequestDetailModal({ isOpen, onClose, detailId = null, detail: detailProp = null }) {
  const [detail, setDetail] = useState(detailProp);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [rawTab, setRawTab] = useState("client"); // client | provider | response

  useEffect(() => {
    if (!isOpen) return undefined;
    if (detailProp) {
      setDetail(detailProp);
      setError("");
      setLoading(false);
      return undefined;
    }
    if (!detailId) {
      setDetail(null);
      setError("No detail id for this request. Enable Observability in Profile, then make a new request.");
      setLoading(false);
      return undefined;
    }
    let cancelled = false;
    setLoading(true);
    setError("");
    fetch(`/api/usage/request-details/${encodeURIComponent(detailId)}`)
      .then(async (res) => {
        const data = await res.json().catch(() => ({}));
        if (!res.ok) throw new Error(data.error || `HTTP ${res.status}`);
        if (!cancelled) setDetail(data.detail);
      })
      .catch((e) => {
        if (!cancelled) {
          setDetail(null);
          setError(e.message || "Failed to load detail");
        }
      })
      .finally(() => {
        setLoading(false);
      });
    return () => { cancelled = true; };
  }, [isOpen, detailId, detailProp]);

  const request = useMemo(() => {
    const primary = detail?.request;
    const fallback = detail?.providerRequest;
    if (primary && typeof primary === "object") {
      const has =
        (Array.isArray(primary.messages) && primary.messages.length) ||
        (Array.isArray(primary.input) && primary.input.length) ||
        (Array.isArray(primary.tools) && primary.tools.length) ||
        (typeof primary.system === "string" && primary.system) ||
        (typeof primary.instructions === "string" && primary.instructions);
      if (has) return primary;
    }
    return fallback || primary || EMPTY_OBJ;
  }, [detail]);
  const systemPrompt = useMemo(() => extractSystemPrompt(request), [request]);
  const messages = useMemo(() => extractMessages(request), [request]);
  const tools = useMemo(() => extractDeclaredTools(request), [request]);
  const activity = useMemo(
    () => extractToolActivity(request, detail?.response || detail?.providerResponse),
    [request, detail]
  );

  const tokens = detail?.tokens || {};
  const input = tokens.prompt_tokens || tokens.input_tokens || 0;
  const output = tokens.completion_tokens || tokens.output_tokens || 0;
  const cached = tokens.cached_tokens || tokens.cache_read_input_tokens || 0;

  const handleExport = useCallback(() => {
    if (!detail) return;
    try {
      const toon = buildSingleRequestToon(detail);
      const id = safeFilenamePart(detail.id || "request", 80);
      downloadTextFile(`9router-request-${id}.toon`, toon);
    } catch (e) {
      console.error("[RequestDetailModal] TOON export failed:", e);
      setError("Failed to export TOON (payload may be too large or circular).");
    }
  }, [detail]);

  return (
    <Modal
      isOpen={isOpen}
      onClose={onClose}
      title="Request detail"
      size="full"
      className="max-h-[90vh] flex flex-col"
      bodyClassName="max-h-[calc(90vh-4rem)] overflow-y-auto custom-scrollbar"
      footer={
        detail && !loading ? (
          <div className="flex w-full items-center justify-between gap-2">
            <p className="text-[11px] text-text-muted truncate hidden sm:block">
              Export is compact TOON (system + messages + tools) — not raw multi-MB JSON.
            </p>
            <Button variant="primary" size="sm" onClick={handleExport} icon="download">
              Export TOON
            </Button>
          </div>
        ) : null
      }
    >
      <div className="flex flex-col gap-3 p-1">
        {loading && (
          <div className="flex items-center gap-2 text-text-muted text-sm py-8 justify-center">
            <span className="material-symbols-outlined animate-spin text-[20px]">progress_activity</span>
            Loading request body…
          </div>
        )}

        {error && !loading && (
          <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-800 dark:text-amber-200">
            {error}
            <p className="text-xs mt-1 opacity-80">
              Dashboard → Profile → Observability must be ON. New requests after that get full body capture
              (default up to ~2MB per field; media redacted).
            </p>
          </div>
        )}

        {detail && !loading && (
          <>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 text-xs flex-1 min-w-0">
                <Meta label="Model" value={detail.model} mono />
                <Meta label="Provider" value={detail.provider} />
                <Meta label="Status" value={detail.status} />
                <Meta label="When" value={detail.timestamp ? new Date(detail.timestamp).toLocaleString() : "—"} />
                <Meta label="Input" value={fmtTokens(input)} mono />
                <Meta label="Cached" value={fmtTokens(cached)} mono />
                <Meta label="Output" value={fmtTokens(output)} mono />
                <Meta
                  label="Latency"
                  value={`TTFT ${detail.latency?.ttft || 0}ms · ${detail.latency?.total || 0}ms`}
                  mono
                />
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={handleExport}
                icon="download"
                className="shrink-0 sm:hidden"
              >
                Export
              </Button>
            </div>
            {detail.id && (
              <p className="text-[10px] font-mono text-text-muted break-all">id: {detail.id}</p>
            )}

            <Section title="System / harness prompt" badge={systemPrompt ? `${systemPrompt.length} ch` : "empty"} defaultOpen={!!systemPrompt}>
              {systemPrompt ? (
                <ScrollPre>{systemPrompt}</ScrollPre>
              ) : (
                <p className="text-sm text-text-muted">No system / instructions field on this request (or it lived only inside messages).</p>
              )}
            </Section>

            <Section title="Messages" badge={`${messages.length}`} defaultOpen>
              <MessagesList messages={messages} />
            </Section>

            <Section title="Declared tools" badge={`${tools.length}`} defaultOpen={tools.length > 0 && tools.length <= 30}>
              {tools.length === 0 ? (
                <p className="text-sm text-text-muted">No tools on this request.</p>
              ) : (
                <div className="overflow-x-auto max-h-64 overflow-y-auto">
                  <table className="w-full text-xs">
                    <thead>
                      <tr className="text-left text-text-muted border-b border-border">
                        <th className="py-1 pr-2">Name</th>
                        <th className="py-1">Description</th>
                      </tr>
                    </thead>
                    <tbody className="divide-y divide-border/60">
                      {tools.map((t) => (
                        <tr key={t.name}>
                          <td className="py-1.5 pr-2 font-mono whitespace-nowrap align-top">{t.name}</td>
                          <td className="py-1.5 text-text-muted align-top">{t.description || "—"}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </Section>

            <Section
              title="Tool call activity"
              badge={activity.summary.length ? `${activity.summary.length} tools` : "none"}
              defaultOpen={activity.summary.length > 0}
            >
              {activity.summary.length === 0 ? (
                <p className="text-sm text-text-muted">
                  No tool_use / tool_result / tool_calls found in this turn. (Quota is still the full request
                  tokens above — tools share one prompt budget.)
                </p>
              ) : (
                <>
                  <div className="overflow-x-auto mb-3">
                    <table className="w-full text-xs">
                      <thead>
                        <tr className="text-left text-text-muted border-b border-border">
                          <th className="py-1 pr-2">Tool</th>
                          <th className="py-1 pr-2 text-right">Calls</th>
                          <th className="py-1 pr-2 text-right">Results</th>
                          <th className="py-1 text-right">Payload size</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-border/60">
                        {activity.summary.map((row) => (
                          <tr key={row.name}>
                            <td className="py-1.5 pr-2 font-mono">{row.name}</td>
                            <td className="py-1.5 pr-2 text-right font-mono">{row.calls}</td>
                            <td className="py-1.5 pr-2 text-right font-mono">{row.results}</td>
                            <td className="py-1.5 text-right font-mono">{formatBytesish(row.chars)}</td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                  <p className="text-[10px] text-text-muted mb-2">
                    Payload size is character count of args/results in this request (proxy for token share).
                    Providers do not bill per-tool; the whole prompt is one input.
                  </p>
                  <div className="flex flex-col gap-1.5 max-h-48 overflow-y-auto">
                    {activity.calls.map((c, i) => (
                      <div key={i} className="text-[11px] font-mono border border-border/70 rounded px-2 py-1">
                        <span className={c.phase === "call" ? "text-primary" : "text-success"}>
                          {c.phase === "call" ? "→ call" : "← result"}
                        </span>{" "}
                        <span className="font-semibold">{c.name}</span>
                        {c.id ? <span className="text-text-muted"> · {c.id}</span> : null}
                        <span className="text-text-muted"> · {formatBytesish(c.chars)}</span>
                        <div className="text-text-muted truncate opacity-80">{c.preview}</div>
                      </div>
                    ))}
                  </div>
                </>
              )}
            </Section>

            <Section title="Raw JSON" defaultOpen={false}>
              <div className="flex gap-1 mb-2">
                {[
                  ["client", "Client request"],
                  ["provider", "Provider request"],
                  ["response", "Response"],
                ].map(([key, label]) => (
                  <button
                    key={key}
                    type="button"
                    onClick={() => setRawTab(key)}
                    className={cn(
                      "px-2 py-1 rounded text-xs font-medium",
                      rawTab === key ? "bg-primary text-white" : "bg-bg-subtle text-text-muted hover:bg-bg-hover"
                    )}
                  >
                    {label}
                  </button>
                ))}
              </div>
              <ScrollPre className="max-h-[min(45vh,400px)]">
                {JSON.stringify(
                  rawTab === "client"
                    ? detail.request
                    : rawTab === "provider"
                      ? detail.providerRequest
                      : detail.response ?? detail.providerResponse,
                  null,
                  2
                )}
              </ScrollPre>
              {(detail.request?._truncated || detail.providerRequest?._truncated) && (
                <p className="text-xs text-amber-600 mt-2">
                  Body was truncated to the storage budget. Raise Profile → Observability max JSON size (KB)
                  if you need the full harness dump.
                </p>
              )}
            </Section>
          </>
        )}
      </div>
    </Modal>
  );
}

function Meta({ label, value, mono }) {
  return (
    <div className="rounded-md border border-border px-2 py-1.5 min-w-0">
      <div className="text-[10px] uppercase tracking-wide text-text-muted">{label}</div>
      <div className={cn("text-xs text-text-main truncate", mono && "font-mono")} title={String(value ?? "")}>
        {value ?? "—"}
      </div>
    </div>
  );
}
