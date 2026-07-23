import { describe, it, expect } from "vitest";
import { sortMemberRows, groupMemberRows, aggregateMemberSummary } from "../../src/app/(dashboard)/dashboard/usage/components/membersTable.js";

const rows = [
  { keyName: "a", model: "opus", cost: 1, requests: 10, meanTPS: 50 },
  { keyName: "b", model: "opus", cost: 5, requests: 2, meanTPS: 80 },
  { keyName: "c", model: "haiku", cost: 3, requests: 7, meanTPS: 100 },
];

describe("sortMemberRows", () => {
  it("sorts by cost desc by default", () => {
    const out = sortMemberRows(rows);
    expect(out.map((r) => r.keyName)).toEqual(["b", "c", "a"]);
  });
  it("sorts by requests asc", () => {
    const out = sortMemberRows(rows, "requests", "asc");
    expect(out.map((r) => r.keyName)).toEqual(["b", "c", "a"]);
  });
  it("sorts by meanTPS desc", () => {
    const out = sortMemberRows(rows, "meanTPS", "desc");
    expect(out.map((r) => r.keyName)).toEqual(["c", "b", "a"]);
  });
});

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
