import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@/lib/usageDb.js", () => ({
  saveRequestUsage: vi.fn(() => Promise.resolve()),
  appendRequestLog: vi.fn(() => Promise.resolve()),
  saveRequestDetail: vi.fn(() => Promise.resolve()),
}));

import { saveUsageStats } from "../../open-sse/handlers/chatCore/requestDetail.js";
import { saveRequestUsage } from "@/lib/usageDb.js";

describe("saveUsageStats — silent param", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("does not throw when silent is omitted (regression: ReferenceError silent is not defined)", () => {
    expect(() =>
      saveUsageStats({
        provider: "glm",
        model: "glm-5.2",
        tokens: { prompt_tokens: 10, completion_tokens: 5 },
        connectionId: "88655f41-52fd-42ee-a38a-6aceda413fed",
        latency: { ttft: 100, total: 500 },
      })
    ).not.toThrow();
    expect(saveRequestUsage).toHaveBeenCalledOnce();
  });

  it("skips console usage line when silent=true but still persists", () => {
    const log = vi.spyOn(console, "log").mockImplementation(() => {});
    saveUsageStats({
      provider: "glm",
      model: "glm-5.2",
      tokens: { prompt_tokens: 10, completion_tokens: 5 },
      silent: true,
    });
    expect(log).not.toHaveBeenCalled();
    expect(saveRequestUsage).toHaveBeenCalledOnce();
    log.mockRestore();
  });
});
