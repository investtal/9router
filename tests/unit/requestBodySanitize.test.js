import { describe, it, expect } from "vitest";
import { sanitizeForStorage } from "../../src/lib/db/repos/requestBodySanitize.js";

describe("sanitizeForStorage", () => {
  it("keeps small system + messages intact", () => {
    const body = {
      model: "glm-5.2",
      system: "You are helpful.",
      messages: [
        { role: "user", content: "hi" },
        { role: "assistant", content: "hello" },
      ],
      tools: [{ name: "Bash", description: "run shell", input_schema: { type: "object" } }],
    };
    const out = sanitizeForStorage(body, 50_000);
    expect(out.system).toBe("You are helpful.");
    expect(out.messages).toHaveLength(2);
    expect(out.tools[0].name).toBe("Bash");
    expect(out._truncated).toBeUndefined();
  });

  it("redacts large base64-like media", () => {
    const b64 = "A".repeat(2000);
    const body = {
      messages: [{
        role: "user",
        content: [{ type: "image", source: { type: "base64", data: b64 } }],
      }],
    };
    const out = sanitizeForStorage(body, 100_000);
    const data = out.messages[0].content[0].source.data;
    expect(data).toMatch(/media omitted/);
  });

  it("drops oldest messages when over budget", () => {
    const messages = Array.from({ length: 40 }, (_, i) => ({
      role: i % 2 ? "assistant" : "user",
      content: `msg-${i}-` + "x".repeat(800),
    }));
    const body = { system: "sys", messages };
    const out = sanitizeForStorage(body, 8_000);
    expect(out.system).toBe("sys");
    expect(out.messages.length).toBeLessThan(40);
    expect(out.messages.length).toBeGreaterThanOrEqual(2);
  });
});
