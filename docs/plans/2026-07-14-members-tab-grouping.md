# Members Tab — Group by Member Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use `subagent-parallel-execution` (recommended) or inline execution via `finishing-execution` to implement task-by-task. Steps use `- [ ]` checkboxes.

**Goal:** Collapse the Members usage tab from one row per `apiKey × model` cell to one expandable summary row per member, with per-model sub-rows revealed on click.

**Architecture:** Pure client-side change. A new `groupMemberRows` helper in `membersTable.js` groups the existing flat `/api/usage/members` response by member `id` and produces a summary (Σ requests/tokens/cost, max lastUsed, TPS null). `MembersTab.js` renders summary rows with an expand/collapse toggle; expanded sub-rows are the original per-model cells. No API or schema change.

**Tech Stack:** React (client component), vitest, plain JS (no TS).

## Global Constraints

- No new dependency. No API change. No schema change.
- `sortMemberRows` behavior stays unchanged — existing tests must remain green.
- Summary TPS cells render `—` (null), never a misleading aggregated number.
- Sub-rows sorted by `cost` desc (matches `byModel` in `/api/usage/members/:id`).
- Test command: `npx vitest run tests/unit/membersTable.test.js`.
- Commits use `IVT-0000` task suffix (no Lark task ID assigned).

---

### Task 1: `groupMemberRows` + `aggregateMemberSummary` helpers (TDD)

**Files:**
- Modify: `src/app/(dashboard)/dashboard/usage/components/membersTable.js`
- Test: `tests/unit/membersTable.test.js`

**Interfaces:**
- Consumes: `sortMemberRows(rows, sortBy, sortDir)` (already exported from same file).
- Produces:
  - `aggregateMemberSummary(cells: Cell[]) => Summary` — same field shape as a cell, with `requests`/`promptTokens`/`completionTokens`/`cachedTokens`/`cost` summed, `lastUsed` = max, TPS fields `null`, `model`/`provider` = `null`, `id`/`keyName`/`apiKeyMasked` from first cell.
  - `groupMemberRows(rows: Cell[]) => Array<{ summary: Summary, cells: Cell[] }>` — groups by `id` (fallback `keyName`, fallback `"__none__"`), `cells` sorted by `cost` desc, groups in first-seen order.

- [ ] **Step 1: Write the failing tests** — append to `tests/unit/membersTable.test.js`:

```js
import { sortMemberRows, groupMemberRows, aggregateMemberSummary } from "../../src/app/(dashboard)/dashboard/usage/components/membersTable.js";

const memberCells = [
  { id: "1", keyName: "alice", model: "opus",  provider: "Anthropic", requests: 80, promptTokens: 2000000, completionTokens: 400000, cachedTokens: 100000, cost: 28.00, meanTPS: 62, p50TPS: 60, p95TPS: 78, throughputTPS: 59, lastUsed: "2026-07-13T09:12:00.000Z" },
  { id: "1", keyName: "alice", model: "haiku", provider: "Anthropic", requests: 62, promptTokens: 1100000, completionTokens:  88000, cachedTokens:  30000, cost:  3.42, meanTPS: 95, p50TPS: 93, p95TPS: 110, throughputTPS: 91, lastUsed: "2026-07-12T18:44:00.000Z" },
  { id: "2", keyName: "bob",   model: "opus",  provider: "Anthropic", requests: 88, promptTokens:  900000, completionTokens: 120000, cachedTokens:  10000, cost: 12.10, meanTPS: 70, p50TPS: 68, p95TPS: 90, throughputTPS: 65, lastUsed: "2026-07-14T01:00:00.000Z" },
];

describe("groupMemberRows", () => {
  it("groups cells by member id", () => {
    const groups = groupMemberRows(memberCells);
    expect(groups).toHaveLength(2);
    expect(groups.map((g) => g.summary.keyName)).toEqual(["alice", "bob"]);
  });

  it("sorts each group's cells by cost desc", () => {
    const groups = groupMemberRows(memberCells);
    const alice = groups.find((g) => g.summary.keyName === "alice");
    expect(alice.cells.map((c) => c.model)).toEqual(["opus", "haiku"]);
  });

  it("summary sums requests, tokens, cost across cells", () => {
    const groups = groupMemberRows(memberCells);
    const alice = groups.find((g) => g.summary.keyName === "alice");
    expect(alice.summary.requests).toBe(142);
    expect(alice.summary.promptTokens).toBe(3100000);
    expect(alice.summary.completionTokens).toBe(488000);
    expect(alice.summary.cachedTokens).toBe(130000);
    expect(alice.summary.cost).toBeCloseTo(31.42, 2);
  });

  it("summary lastUsed is the max across cells", () => {
    const groups = groupMemberRows(memberCells);
    const alice = groups.find((g) => g.summary.keyName === "alice");
    expect(alice.summary.lastUsed).toBe("2026-07-13T09:12:00.000Z");
  });

  it("summary TPS fields are null", () => {
    const groups = groupMemberRows(memberCells);
    const alice = groups.find((g) => g.summary.keyName === "alice");
    expect(alice.summary.meanTPS).toBeNull();
    expect(alice.summary.p50TPS).toBeNull();
    expect(alice.summary.p95TPS).toBeNull();
    expect(alice.summary.throughputTPS).toBeNull();
  });

  it("falls back to keyName when id is missing", () => {
    const noId = [
      { keyName: "carol", model: "opus",  cost: 5, requests: 1, lastUsed: "2026-07-14T00:00:00.000Z" },
      { keyName: "carol", model: "haiku", cost: 2, requests: 1, lastUsed: "2026-07-14T00:00:00.000Z" },
    ];
    const groups = groupMemberRows(noId);
    expect(groups).toHaveLength(1);
    expect(groups[0].cells).toHaveLength(2);
  });

  it("handles null/undefined input", () => {
    expect(groupMemberRows(null)).toEqual([]);
    expect(groupMemberRows(undefined)).toEqual([]);
  });
});

describe("aggregateMemberSummary", () => {
  it("sums a single member's cells and takes max lastUsed", () => {
    const summary = aggregateMemberSummary([
      { id: "1", keyName: "alice", requests: 10, promptTokens: 100, completionTokens: 20, cachedTokens: 5, cost: 1.5, lastUsed: "2026-07-13T09:00:00.000Z" },
      { id: "1", keyName: "alice", requests: 5,  promptTokens: 50,  completionTokens: 10, cachedTokens: 2, cost: 0.5, lastUsed: "2026-07-14T09:00:00.000Z" },
    ]);
    expect(summary.requests).toBe(15);
    expect(summary.promptTokens).toBe(150);
    expect(summary.completionTokens).toBe(30);
    expect(summary.cachedTokens).toBe(7);
    expect(summary.cost).toBeCloseTo(2.0, 2);
    expect(summary.lastUsed).toBe("2026-07-14T09:00:00.000Z");
    expect(summary.meanTPS).toBeNull();
    expect(summary.throughputTPS).toBeNull();
    expect(summary.model).toBeNull();
  });
});
```

  Note: the existing `import { sortMemberRows } from ...` line at the top of the file must be replaced by the combined import line above (single import statement, three named exports).

- [ ] **Step 2: Run tests, verify they fail** — Run: `npx vitest run tests/unit/membersTable.test.js` Expected: FAIL — `groupMemberRows is not a function` / `aggregateMemberSummary is not a function`. Existing 3 `sortMemberRows` tests still PASS.

- [ ] **Step 3: Write minimal implementation** — append to `src/app/(dashboard)/dashboard/usage/components/membersTable.js` (after the existing `sortMemberRows`):

```js
// Aggregate a member's per-model cells into one summary row.
// TPS fields are null: mean/p50/p95 need per-request samples and throughput
// needs raw Σ genSeconds — none derivable from per-cell stats.
// ponytail: swap to /api/usage/members/:id totals for exact aggregated TPS.
export function aggregateMemberSummary(cells) {
  const summary = {
    id: cells[0]?.id,
    keyName: cells[0]?.keyName,
    apiKeyMasked: cells[0]?.apiKeyMasked,
    model: null,
    provider: null,
    requests: 0,
    promptTokens: 0,
    completionTokens: 0,
    cachedTokens: 0,
    cost: 0,
    meanTPS: null,
    p50TPS: null,
    p95TPS: null,
    throughputTPS: null,
    lastUsed: null,
  };
  for (const c of cells) {
    summary.requests += c.requests || 0;
    summary.promptTokens += c.promptTokens || 0;
    summary.completionTokens += c.completionTokens || 0;
    summary.cachedTokens += c.cachedTokens || 0;
    summary.cost += c.cost || 0;
    if (c.lastUsed && c.lastUsed > summary.lastUsed) summary.lastUsed = c.lastUsed;
  }
  return summary;
}

// Group flat member×model cells into one group per member.
// Group key: id (apiKeys.id uuid), fallback keyName. cells sorted by cost desc.
export function groupMemberRows(rows) {
  const order = [];
  const byKey = new Map();
  for (const r of rows || []) {
    const key = r.id || r.keyName || "__none__";
    if (!byKey.has(key)) {
      byKey.set(key, []);
      order.push(key);
    }
    byKey.get(key).push(r);
  }
  return order.map((key) => {
    const cells = sortMemberRows(byKey.get(key), "cost", "desc");
    return { summary: aggregateMemberSummary(cells), cells };
  });
}
```

- [ ] **Step 4: Run tests, verify pass** — Run: `npx vitest run tests/unit/membersTable.test.js` Expected: PASS (all tests, including the original 3 `sortMemberRows` cases).

- [ ] **Step 5: Commit** — `git add src/app/\(dashboard\)/dashboard/usage/components/membersTable.js tests/unit/membersTable.test.js && git commit -m "feat(usage): groupMemberRows + aggregateMemberSummary helpers IVT-0000"`

---

### Task 2: Render grouped expandable rows in `MembersTab.js`

**Files:**
- Modify: `src/app/(dashboard)/dashboard/usage/components/MembersTab.js`

**Interfaces:**
- Consumes: `groupMemberRows(rows)` and `sortMemberRows(rows, sortBy, sortDir)` from `./membersTable.js` (Task 1).
- Produces: a Members tab where each member is one summary row; clicking the row toggles a set of indented per-model sub-rows with full TPS columns.

- [ ] **Step 1: Replace `MembersTab.js` with the grouped implementation** — write the full file:

```js
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
  return v ? <span suppressHydrationWarning>{new Date(v).toLocaleString()}</span> : "—";
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
    const byKey = new Map(groups.map((g) => [g.summary.id || g.summary.keyName, g]));
    return order.map((s) => byKey.get(s.id || s.keyName));
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
```

- [ ] **Step 2: Run unit tests, verify still green** — Run: `npx vitest run tests/unit/membersTable.test.js` Expected: PASS (no behavioral change to helpers; regression guard).

- [ ] **Step 3: Verify build compiles** — Run: `npx vinext build` Expected: build succeeds with no type/compile errors. (Skip if build is too slow in the execution environment; a `node --check` on the file is an acceptable lighter proxy: `node --check "src/app/(dashboard)/dashboard/usage/components/MembersTab.js"`.)

- [ ] **Step 4: Commit** — `git add "src/app/(dashboard)/dashboard/usage/components/MembersTab.js" && git commit -m "feat(usage): group Members tab rows by member with expandable models IVT-0000"`

---

## Self-Review

**Spec coverage:**
- "No API change. Group client-side." → Task 1 + 2, no route edits. ✓
- `aggregateMemberSummary` sums + max lastUsed + TPS null → Task 1 tests + impl. ✓
- `groupMemberRows` by id, fallback keyName, cells cost desc → Task 1 tests + impl. ✓
- Summary row: ▼/▶ keyName, sums, TPS `—` → Task 2. ✓
- Sub-rows: full TPS, indented under Model → Task 2. ✓
- Sort: summaries via `sortMemberRows`, sub-rows cost desc → Task 2 `sortedGroups` + Task 1 cells sort. ✓
- Collapsed by default → Task 2 `useState(() => new Set())`. ✓
- Tests: grouping, aggregation, fallback, null input → Task 1. ✓

**Placeholder scan:** none. All code blocks complete.

**Type/name consistency:** `groupMemberRows`, `aggregateMemberSummary`, `sortMemberRows` — names match across spec, Task 1, Task 2. `summary.id || summary.keyName` keying consistent in Task 2. `lastUsedCell` helper defined in Task 2 before use.

**Risk:** low. Two files, pure UI + pure helpers. No API/schema touch. Existing tests guard `sortMemberRows` regression.
