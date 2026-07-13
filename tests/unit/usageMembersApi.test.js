import { describe, it, expect, vi, beforeEach } from "vitest";

// Single mock for the whole file — covers both list + detail route tests.
vi.mock("@/lib/usageDb", () => ({
  getMemberStats: vi.fn(),
  getMemberDetail: vi.fn(),
}));
import { getMemberStats, getMemberDetail } from "@/lib/usageDb";

describe("GET /api/usage/members", () => {
  beforeEach(() => {
    vi.mocked(getMemberStats).mockResolvedValue([
      { id: "u1", apiKey: "sk-secret", apiKeyMasked: "sk-sec***", keyName: "alice",
        model: "opus", provider: "Anthropic", requests: 2, promptTokens: 10,
        completionTokens: 20, cachedTokens: 0, cost: 1.5,
        meanTPS: 50, p50TPS: 50, p95TPS: 60, throughputTPS: 48, lastUsed: "2026-07-13T00:00:00.000Z" },
    ]);
    vi.mocked(getMemberDetail).mockResolvedValue(null);
  });

  it("returns json without raw apiKey", async () => {
    const { GET } = await import("../../src/app/api/usage/members/route.js");
    const req = new Request("http://x/api/usage/members?period=7d");
    const res = await GET(req);
    expect(res.status).toBe(200);
    const body = await res.json();
    expect(body.period).toBe("7d");
    expect(body.members[0].apiKey).toBeUndefined();
    expect(body.members[0].apiKeyMasked).toBe("sk-sec***");
    expect(body.members[0].id).toBe("u1");
  });

  it("rejects invalid period with 400", async () => {
    const { GET } = await import("../../src/app/api/usage/members/route.js");
    const res = await GET(new Request("http://x/api/usage/members?period=bogus"));
    expect(res.status).toBe(400);
  });

  it("returns csv when format=csv", async () => {
    const { GET } = await import("../../src/app/api/usage/members/route.js");
    const res = await GET(new Request("http://x/api/usage/members?period=all&format=csv"));
    expect(res.headers.get("content-type")).toContain("text/csv");
    const text = await res.text();
    expect(text.split("\n")[0]).toContain("keyName");
    expect(text).toContain("alice");
    expect(text).not.toContain("sk-secret");
  });
});
