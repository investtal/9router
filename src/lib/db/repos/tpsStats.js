// Pure TPS statistics helpers. No DB access — shared by getMemberStats and
// getMemberDetail so list and detail cannot drift.

export function nearestRank(sortedAsc, percentile) {
  if (!Array.isArray(sortedAsc) || sortedAsc.length === 0) return null;
  const n = sortedAsc.length;
  const rank = Math.ceil((percentile / 100) * n);
  const idx = Math.min(Math.max(rank, 1), n) - 1;
  return sortedAsc[idx];
}

export function computeTpsStats(rows) {
  const out = { meanTPS: null, p50TPS: null, p95TPS: null, throughputTPS: null, sampleCount: 0 };
  if (!Array.isArray(rows) || rows.length === 0) return out;

  const samples = [];
  let sumCompletion = 0;
  let sumGenSeconds = 0;
  for (const r of rows) {
    const completion = r?.completionTokens || 0;
    const total = r?.latencyTotalMs || 0;
    if (completion <= 0 || total <= 0) continue;
    const ttft = r?.latencyTtftMs || 0;
    const genMs = Math.max(total - ttft, 1);
    const genSeconds = genMs / 1000;
    samples.push(completion / genSeconds);
    sumCompletion += completion;
    sumGenSeconds += genSeconds;
  }

  if (samples.length === 0) return out;
  samples.sort((a, b) => a - b);
  const mean = samples.reduce((s, v) => s + v, 0) / samples.length;
  return {
    meanTPS: mean,
    p50TPS: nearestRank(samples, 50),
    p95TPS: nearestRank(samples, 95),
    throughputTPS: sumGenSeconds > 0 ? sumCompletion / sumGenSeconds : null,
    sampleCount: samples.length,
  };
}
