import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";

const originalDataDir = process.env.DATA_DIR;
let tempDir;

beforeAll(async () => {
  tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "9router-mig-002-"));
  process.env.DATA_DIR = tempDir;
  vi.resetModules();
});

afterAll(() => {
  if (tempDir) fs.rmSync(tempDir, { recursive: true, force: true });
  if (originalDataDir === undefined) delete process.env.DATA_DIR;
  else process.env.DATA_DIR = originalDataDir;
});

describe("migration 002 usage-latency", () => {
  it("adds latency columns + index and stamps schemaVersion=2", async () => {
    const { getAdapter } = await import("@/lib/db/driver.js");
    const db = await getAdapter();

    const cols = db.all(`PRAGMA table_info(usageHistory)`).map((r) => r.name);
    expect(cols).toContain("latencyTotalMs");
    expect(cols).toContain("latencyTtftMs");

    const idxs = db.all(`PRAGMA index_list(usageHistory)`).map((r) => r.name);
    expect(idxs).toContain("idx_uh_key_model_ts");

    const { getMetaSync } = await import("@/lib/db/helpers/metaStore.js");
    expect(getMetaSync(db, "schemaVersion", "0")).toBe("2");
  });

  it("inserts a usage row with default latency 0", async () => {
    const { getAdapter } = await import("@/lib/db/driver.js");
    const db = await getAdapter();
    db.run(
      `INSERT INTO usageHistory(timestamp, provider, model, apiKey, promptTokens, completionTokens) VALUES(?, ?, ?, ?, ?, ?)`,
      [new Date().toISOString(), "anthropic", "claude-opus-4-8", "sk-x", 10, 20]
    );
    const row = db.get(`SELECT latencyTotalMs, latencyTtftMs FROM usageHistory WHERE apiKey = 'sk-x'`);
    expect(row.latencyTotalMs).toBe(0);
    expect(row.latencyTtftMs).toBe(0);
  });
});
