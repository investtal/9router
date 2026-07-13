import { describe, it, expect } from "vitest";
import { nearestRank, computeTpsStats } from "../../src/lib/db/repos/tpsStats.js";

describe("nearestRank", () => {
  it("returns null for empty input", () => {
    expect(nearestRank([], 50)).toBeNull();
  });
  it("computes p50 (median of 5)", () => {
    expect(nearestRank([10, 20, 30, 40, 50], 50)).toBe(30);
  });
  it("computes p95 via nearest-rank", () => {
    // n=10, rank=ceil(0.95*10)=10 -> last element
    expect(nearestRank([1, 2, 3, 4, 5, 6, 7, 8, 9, 100], 95)).toBe(100);
  });
});

describe("computeTpsStats", () => {
  it("returns nulls when no qualifying rows", () => {
    const r = computeTpsStats([{ completionTokens: 0, latencyTotalMs: 100, latencyTtftMs: 10 }]);
    expect(r.sampleCount).toBe(0);
    expect(r.meanTPS).toBeNull();
    expect(r.p95TPS).toBeNull();
    expect(r.throughputTPS).toBeNull();
  });
  it("computes mean, p50, p95, throughput", () => {
    // genMs = total - ttft; reqTPS = completion / (genMs/1000)
    const rows = [
      { completionTokens: 600, latencyTotalMs: 1100, latencyTtftMs: 100 }, // gen 1000ms -> 600 tps
      { completionTokens: 300, latencyTotalMs: 600, latencyTtftMs: 100 },  // gen 500ms  -> 600 tps
      { completionTokens: 1000, latencyTotalMs: 2100, latencyTtftMs: 100 },// gen 2000ms -> 500 tps
    ];
    const r = computeTpsStats(rows);
    expect(r.sampleCount).toBe(3);
    expect(r.meanTPS).toBeCloseTo((600 + 600 + 500) / 3, 5);       // ~566.67
    expect(r.p50TPS).toBeCloseTo(600, 5);                           // sorted [500,600,600]
    expect(r.p95TPS).toBeCloseTo(600, 5);                           // ceil(0.95*3)=3 -> 600
    // throughput = sum(completion)/sum(genSeconds) = 1900 / (1+0.5+2) = 542.857
    expect(r.throughputTPS).toBeCloseTo(1900 / 3.5, 5);
  });
  it("skips degenerate samples where ttft >= total (no real TTFT captured)", () => {
    // ttft >= total would clamp genMs to 1ms and inflate TPS ~1000x; guard rejects the sample.
    const r = computeTpsStats([
      { completionTokens: 100, latencyTotalMs: 50, latencyTtftMs: 80 },   // ttft > total
      { completionTokens: 100, latencyTotalMs: 50, latencyTtftMs: 50 },   // ttft == total boundary
    ]);
    expect(r.sampleCount).toBe(0);
    expect(r.meanTPS).toBeNull();
    expect(r.throughputTPS).toBeNull();
  });
});
