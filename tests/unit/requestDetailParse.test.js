import { describe, it, expect } from "vitest";
import {
  extractSystemPrompt,
  extractMessages,
  extractDeclaredTools,
  extractToolActivity,
  aggregateToolStats,
} from "../../src/shared/utils/requestDetailParse.js";

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
    expect(extractMessages(req)).toHaveLength(3);
    expect(extractDeclaredTools(req)[0].name).toBe("Bash");
    const act = extractToolActivity(req, {});
    expect(act.summary.some((s) => s.name === "Bash" && s.calls >= 1)).toBe(true);
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
});
