# Members Tab — Group by Member Design

## Purpose

Reduce confusion on `/dashboard/usage?tab=members` when one member uses many models. Today the tab renders one row per `apiKey × model` cell, so a member using 10 models produces 10 rows with the same member name repeated. Group those rows by member so each member is one summary row, expandable to reveal the per-model matrix.

## Context

- `/api/usage/members?period=` returns a flat `members[]` array, one entry per `apiKey × model × provider` cell, with `id` (apiKeys.id uuid), `keyName`, `model`, `provider`, `requests`, `promptTokens`, `completionTokens`, `cachedTokens`, `cost`, `meanTPS`, `p50TPS`, `p95TPS`, `throughputTPS`, `lastUsed`.
- `MembersTab.js` renders that flat array as a sortable table.
- `membersTable.js` exposes `sortMemberRows(rows, sortBy, sortDir)` — a pure sort helper.
- Identity model (from prior spec): one API key = one member. `id` is the stable member identifier; `keyName` is the display name.

## Constraints

- No API change. Group client-side from the existing flat response.
- No new dependency.
- Aggregated sums on the summary row must be exact (simple addition over cells).
- TPS statistics (mean / p50 / p95 / throughput) cannot be aggregated correctly from per-cell stats alone — they require per-request samples. Summary TPS cells render `—`, not a misleading number.
- Sub-rows preserve the existing per-cell TPS values verbatim from the API.
- Collapsed by default; user expands on demand.

## Success Criteria

- A member using N models renders as exactly one summary row when collapsed, instead of N rows.
- Expanding a member reveals N sub-rows (one per model) with full TPS columns.
- Summary row sums (requests, input, output, cost) match the sum of that member's expanded sub-rows.
- Summary `lastUsed` equals the max `lastUsed` across the member's cells.
- Sorting the table sorts summary groups; expanded sub-rows stay sorted by cost desc.
- Existing flat `sortMemberRows` behavior and unit tests remain green.

## Decisions

| Trade-off | Chosen Option | Why |
|---|---|---|
| Where grouping lives | Client-side, in `membersTable.js` | Flat API already has all data; no server change, no extra fetch. |
| Summary TPS cells | Render `—` | mean/p50/p95 need per-request samples; throughput needs Σ genSeconds not in response. A wrong number is worse than none. |
| Exact-correct aggregated TPS later | `ponytail:` call `/api/usage/members/:id` per member on expand (already exists, returns server-computed `totals`) | Upgrade path already built; defer until asked. |
| Sub-row sort | Fixed cost desc | Matches `byModel` in the detail endpoint; predictable. `ponytail:` honor summary sort key later. |
| Group key | `id` (uuid), fallback `keyName` | `id` is the stable member identity; `keyName` is display only. |
| Default expand state | All collapsed | Lowest noise; drill on demand. |

## Components

### `src/app/(dashboard)/dashboard/usage/components/membersTable.js`

Add two pure helpers alongside the existing `sortMemberRows`:

- `aggregateMemberSummary(cells)` → returns an object with the same field shape as a cell (`keyName`, `id`, `requests`, `promptTokens`, `completionTokens`, `cachedTokens`, `cost`, `meanTPS: null`, `p50TPS: null`, `p95TPS: null`, `throughputTPS: null`, `lastUsed`) where:
  - `requests`, `promptTokens`, `completionTokens`, `cachedTokens`, `cost` = Σ across cells.
  - `lastUsed` = max `lastUsed` across cells (lexicographic ISO max = latest).
  - TPS fields = `null`.
  - `model` / `provider` omitted (summary spans multiple models).

- `groupMemberRows(rows)` → returns `[{ summary, cells }]`:
  - Group by `id` (fallback `keyName` when `id` missing).
  - Preserve first-seen order of groups (stable; final order comes from the sort applied after).
  - `cells` sorted by `cost` desc.
  - `summary` = `aggregateMemberSummary(cells)`, with `keyName` and `id` taken from the first cell.

`sortMemberRows` is reused unchanged to sort the array of summary objects — they share the same field names, and `null` TPS fields sort last per the existing null guard.

### `src/app/(dashboard)/dashboard/usage/components/MembersTab.js`

- Compute `const groups = useMemo(() => groupMemberRows(rows), [rows])`.
- Sort summaries: `const sortedGroups = useMemo(() => sortMemberRows(groups.map(g => g.summary), sortBy, sortDir), [groups, sortBy, sortDir])`, then map each sorted summary back to its `cells`.
- `expanded` state: `useState(() => new Set())`. Toggle on summary-row click.
- Render one `<tbody>` with, per group: a summary row (Member cell shows `▼`/`▶` + `keyName`; Model cell blank; summed columns; TPS cells `—`) followed by expanded sub-rows when open (Model cell = model id, indented; full per-cell TPS).
- Sub-rows reuse `fmt` and the existing `lastUsed` rendering.
- Column headers, sort toggle, and the TPS footnote stay as-is.

### `tests/unit/membersTable.test.js`

- `groupMemberRows`: flat cells for 2 members (one with 2 models, one with 1) → 2 groups; group `cells` sorted by cost desc; group `summary` sums and `lastUsed` max correct; TPS summary fields `null`.
- `aggregateMemberSummary`: single-member input → sums match; `lastUsed` = max.
- `groupMemberRows` with `id` missing falls back to `keyName` grouping.
- Existing `sortMemberRows` tests remain green (regression guard).

One assert-based self-check for the grouping helper is acceptable in lieu of a framework test only if the existing vitest file is unavailable; otherwise land the vitest cases above.

## Error Handling

- No new error paths. Fetch/error/empty/loading states in `MembersTab.js` are unchanged.
- If a cell lacks `id` and `keyName`, it forms its own singleton group — never throws.

## Out of Scope

- Server-side grouped API response.
- Aggregated TPS on the summary row (upgrade path: per-member detail endpoint on expand).
- Per-group sub-row sort honoring the summary sort key.
- Custom date-range picker, CSV export changes.
- Multi-key member grouping / owner field.
