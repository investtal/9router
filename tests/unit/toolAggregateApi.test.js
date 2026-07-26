import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { describe, it, expect, beforeAll, afterAll, vi } from "vitest";

const originalDataDir = process.env.DATA_DIR;
let tempDir;
let db;

async function saveDetail(detail) {
  await db.saveRequestDetail(detail);
  await new Promise((r) => setTimeout(r, 80));
}

beforeAll(async () => {
  tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "9router-tools-agg-"));
  process.env.DATA_DIR = tempDir;
  vi.resetModules();
  db = await import("@/lib/db/index.js");
  await db.initDb();
  await db.updateSettings({ enableObservability: true, observabilityBatchSize: 1 });
});

afterAll(() => {
  if (tempDir) fs.rmSync(tempDir, { recursive: true, force: true });
  if (originalDataDir === undefined) delete process.env.DATA_DIR;
  else process.env.DATA_DIR = originalDataDir;
});

describe("getToolAggregateStats", () => {
  it("rolls up tool calls from stored details", async () => {
    const ts = new Date().toISOString();
    await saveDetail({
      id: "tool-agg-1",
      timestamp: ts,
      provider: "glm",
      model: "glm-5.2",
      status: "success",
      request: {
        messages: [
          {
            role: "assistant",
            content: [{ type: "tool_use", id: "t1", name: "Grep", input: { pattern: "foo" } }],
          },
          {
            role: "user",
            content: [{ type: "tool_result", tool_use_id: "t1", content: "match line" }],
          },
        ],
      },
      response: { content: "done" },
    });

    const res = await db.getToolAggregateStats({ period: "all" });
    expect(res.scanned).toBeGreaterThanOrEqual(1);
    const grep = res.tools.find((t) => t.name === "Grep");
    expect(grep).toBeDefined();
    expect(grep.calls).toBeGreaterThanOrEqual(1);
    expect(grep.results).toBeGreaterThanOrEqual(1);
    expect(grep.chars).toBeGreaterThan(0);
  });
});

describe("GET /api/usage/tools", () => {
  it("rejects invalid period", async () => {
    const { GET } = await import("@/app/api/usage/tools/route.js");
    const req = new Request("http://localhost/api/usage/tools?period=nope");
    const res = await GET(req);
    expect(res.status).toBe(400);
  });

  it("returns tools array for valid period", async () => {
    const { GET } = await import("@/app/api/usage/tools/route.js");
    const req = new Request("http://localhost/api/usage/tools?period=all");
    const res = await GET(req);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(Array.isArray(body.tools)).toBe(true);
    expect(body.period).toBe("all");
  });
});
