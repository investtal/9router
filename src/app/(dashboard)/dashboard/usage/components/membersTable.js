// Pure sort helper, extracted so it is testable without a DOM.
export function sortMemberRows(rows, sortBy = "cost", sortDir = "desc") {
  const copy = [...rows];
  copy.sort((a, b) => {
    const av = a[sortBy];
    const bv = b[sortBy];
    if (av === null || av === undefined) return bv === null || bv === undefined ? 0 : 1;
    if (bv === null || bv === undefined) return -1;
    if (typeof av === "string" && typeof bv === "string") {
      return sortDir === "asc" ? av.localeCompare(bv) : bv.localeCompare(av);
    }
    if (typeof av !== typeof bv) return 0; // mixed types: leave relative order
    return sortDir === "asc" ? av - bv : bv - av;
  });
  return copy;
}

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
    if (c.lastUsed && (!summary.lastUsed || c.lastUsed > summary.lastUsed)) summary.lastUsed = c.lastUsed;
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
