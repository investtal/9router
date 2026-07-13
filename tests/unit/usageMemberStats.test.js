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
