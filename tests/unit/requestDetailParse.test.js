import { describe, it, expect } from "vitest";
import {
  extractSystemPrompt,
  extractMessages,
  extractDeclaredTools,
  extractToolActivity,
  aggregateToolStats,
  buildExportableDetail,
} from "../../src/shared/utils/requestDetailParse.js";
import { buildSingleRequestToon, buildBulkRequestsToon } from "../../src/shared/utils/toonExport.js";

describe("requestDetailParse", () => {
  it("extracts Claude-style system + tools", () => {
    const req = {
      system: "harness prompt",
      messages: [
        { role: "user", content: "do it" },
        {
          role: "assistant",
          content: [{ type: "tool_use", id: "1", name: "Bash", input: { command: "ls" } }],
        },
        {
          role: "user",
          content: [{ type: "tool_result", tool_use_id: "1", content: "a\nb" }],
        },
      ],
      tools: [{ name: "Bash", description: "shell" }],
    };
    expect(extractSystemPrompt(req)).toBe("harness prompt");
    const msgs = extractMessages(req);
    expect(msgs).toHaveLength(3);
    expect(msgs[0].content).toBe("do it");
    expect(msgs[0].preview).toContain("do it");
    expect(msgs[1].content).toContain("[tool_use Bash]");
    expect(extractDeclaredTools(req)[0].name).toBe("Bash");
    const act = extractToolActivity(req, {});
    expect(act.summary.some((s) => s.name === "Bash" && s.calls >= 1)).toBe(true);
  });

  it("surfaces OpenAI tool_calls when content is empty", () => {
    const msgs = extractMessages({
      messages: [
        {
          role: "assistant",
          content: null,
          tool_calls: [{
            id: "c1",
            function: { name: "read_file", arguments: "{\"path\":\"a.js\"}" },
          }],
        },
      ],
    });
    expect(msgs).toHaveLength(1);
    expect(msgs[0].content).toContain("[tool_call read_file");
    expect(msgs[0].content).toContain("a.js");
  });

  it("extracts thinking blocks", () => {
    const msgs = extractMessages({
      messages: [{
        role: "assistant",
        content: [{ type: "thinking", thinking: "plan the fix" }, { type: "text", text: "done" }],
      }],
    });
    expect(msgs[0].content).toContain("[thinking] plan the fix");
    expect(msgs[0].content).toContain("done");
  });

  it("extracts OpenAI tool_calls", () => {
    const req = {
      messages: [
        {
          role: "assistant",
          tool_calls: [{
            id: "c1",
            function: { name: "read_file", arguments: "{\"path\":\"a.js\"}" },
          }],
        },
        { role: "tool", tool_call_id: "c1", name: "read_file", content: "code" },
      ],
    };
    const act = extractToolActivity(req, null);
    expect(act.summary.find((s) => s.name === "read_file")?.calls).toBe(1);
    expect(act.summary.find((s) => s.name === "read_file")?.results).toBe(1);
  });

  it("aggregates tools across multiple requests", () => {
    const details = [
      {
        request: {
          messages: [
            {
              role: "assistant",
              content: [{ type: "tool_use", id: "1", name: "Bash", input: { command: "ls" } }],
            },
            {
              role: "user",
              content: [{ type: "tool_result", tool_use_id: "1", content: "a\nb\nc" }],
            },
          ],
          tools: [{ name: "Bash" }, { name: "Read" }],
        },
      },
      {
        request: {
          messages: [
            {
              role: "assistant",
              tool_calls: [{ id: "c2", function: { name: "Bash", arguments: "{}" } }],
            },
          ],
        },
      },
    ];
    const agg = aggregateToolStats(details);
    expect(agg.scanned).toBe(2);
    expect(agg.withActivity).toBe(2);
    const bash = agg.tools.find((t) => t.name === "Bash");
    expect(bash.calls).toBe(2);
    expect(bash.requestCount).toBe(2);
    expect(bash.chars).toBeGreaterThan(0);
    const read = agg.tools.find((t) => t.name === "Read");
    expect(read?.declaredOnly).toBe(true);
  });

  it("buildExportableDetail compact shape", () => {
    const compact = buildExportableDetail({
      id: "abc",
      model: "glm-5.2",
      provider: "glm",
      status: "success",
      timestamp: "2026-07-27T00:00:00.000Z",
      tokens: { prompt_tokens: 10, completion_tokens: 3, cached_tokens: 2 },
      latency: { ttft: 1, total: 2 },
      request: {
        system: "sys",
        messages: [{ role: "user", content: "hi" }],
        tools: [{ name: "Bash", description: "shell" }],
      },
    });
    expect(compact.id).toBe("abc");
    expect(compact.tokens.input).toBe(10);
    expect(compact.system).toBe("sys");
    expect(compact.messages[0].content).toBe("hi");
    expect(compact.tools[0].name).toBe("Bash");
  });

  it("encodes single + bulk exports as TOON", () => {
    const detail = {
      id: "r1",
      model: "m",
      provider: "p",
      request: { messages: [{ role: "user", content: "hello agent" }] },
      tokens: { prompt_tokens: 1, completion_tokens: 1 },
    };
    const single = buildSingleRequestToon(detail);
    expect(single).toContain("format: toon");
    expect(single).toContain("hello agent");
    const bulk = buildBulkRequestsToon({ period: "today", details: [detail] });
    expect(bulk).toContain("period: today");
    expect(bulk).toContain("count: 1");
    expect(bulk).toContain("hello agent");
  });
});
