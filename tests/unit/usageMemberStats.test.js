import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";

const originalDataDir = process.env.DATA_DIR;
let tempDir;

beforeAll(async () => {
  tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "9router-writer-"));
  process.env.DATA_DIR = tempDir;
  vi.resetModules();
});

afterAll(() => {
  if (tempDir) fs.rmSync(tempDir, { recursive: true, force: true });
  if (originalDataDir === undefined) delete process.env.DATA_DIR;
  else process.env.DATA_DIR = originalDataDir;
});

describe("saveRequestUsage latency persistence", () => {
  it("stores latencyTotalMs and latencyTtftMs", async () => {
    const { saveRequestUsage } = await import("@/lib/db/index.js");
    const { getAdapter } = await import("@/lib/db/driver.js");
    await saveRequestUsage({
      provider: "anthropic", model: "claude-opus-4-8",
      tokens: { prompt_tokens: 100, completion_tokens: 200 },
      apiKey: "sk-w", latencyTotalMs: 2100, latencyTtftMs: 100,
    });
    const db = await getAdapter();
    const row = db.get(`SELECT latencyTotalMs, latencyTtftMs FROM usageHistory WHERE apiKey = 'sk-w'`);
    expect(row.latencyTotalMs).toBe(2100);
    expect(row.latencyTtftMs).toBe(100);
  });

  it("defaults latency to 0 when not provided", async () => {
    const { saveRequestUsage } = await import("@/lib/db/index.js");
    const { getAdapter } = await import("@/lib/db/driver.js");
    await saveRequestUsage({
      provider: "openai", model: "gpt-4o",
      tokens: { prompt_tokens: 5, completion_tokens: 7 }, apiKey: "sk-no-lat",
    });
    const db = await getAdapter();
    const row = db.get(`SELECT latencyTotalMs, latencyTtftMs FROM usageHistory WHERE apiKey = 'sk-no-lat'`);
    expect(row.latencyTotalMs).toBe(0);
    expect(row.latencyTtftMs).toBe(0);
  });
});

describe("getMemberStats", () => {
  it("aggregates per apiKey x model with TPS stats", async () => {
    const { saveRequestUsage, getMemberStats } = await import("@/lib/db/index.js");
    const base = {
      provider: "anthropic", model: "claude-opus-4-8",
      tokens: { prompt_tokens: 100, completion_tokens: 600 },
      apiKey: "sk-alice", latencyTotalMs: 1100, latencyTtftMs: 100,
    };
    await saveRequestUsage(base);
    await saveRequestUsage({ ...base, tokens: { prompt_tokens: 50, completion_tokens: 300 }, latencyTotalMs: 600, latencyTtftMs: 100 });

    const rows = await getMemberStats("all");
    const cell = rows.find((r) => r.apiKey === "sk-alice" && r.model === "claude-opus-4-8");
    expect(cell).toBeDefined();
    expect(cell.requests).toBe(2);
    expect(cell.promptTokens).toBe(150);
    expect(cell.completionTokens).toBe(900);
    expect(cell.meanTPS).toBeCloseTo((600 + 600) / 2, 3);
    expect(cell.p50TPS).toBeCloseTo(600, 3);
    expect(cell.throughputTPS).toBeCloseTo(900 / 1.5, 3); // genSeconds sum = 1.0 + 0.5
  });

  it("excludes zero-latency rows from TPS but counts them", async () => {
    const { saveRequestUsage, getMemberStats } = await import("@/lib/db/index.js");
    await saveRequestUsage({
      provider: "openai", model: "gpt-4o",
      tokens: { prompt_tokens: 10, completion_tokens: 20 },
      apiKey: "sk-bob",
    });
    const rows = await getMemberStats("all");
    const cell = rows.find((r) => r.apiKey === "sk-bob");
    expect(cell.requests).toBe(1);
    expect(cell.meanTPS).toBeNull();
    expect(cell.p95TPS).toBeNull();
  });

  it("exposes masked key in masked field", async () => {
    const { saveRequestUsage, getMemberStats } = await import("@/lib/db/index.js");
    await saveRequestUsage({
      provider: "anthropic", model: "claude-haiku-4-5-20251001",
      tokens: { prompt_tokens: 1, completion_tokens: 1 },
      apiKey: "sk-1234567890abcdef", latencyTotalMs: 100, latencyTtftMs: 10,
    });
    const rows = await getMemberStats("all");
    const cell = rows.find((r) => r.model === "claude-haiku-4-5-20251001");
    expect(cell.apiKeyMasked).toBe("sk-12345***");
  });
});
