import { describe, expect, it } from "vitest";

import { createPassthroughStreamWithLogger } from "../../open-sse/utils/stream.js";
import { FORMATS } from "../../open-sse/translator/formats.js";

const enc = new TextEncoder();

async function pipe(inputChunks, sourceFormat) {
  const upstream = new ReadableStream({
    start(controller) {
      for (const c of inputChunks) controller.enqueue(enc.encode(c));
      controller.close();
    }
  });

  const transform = createPassthroughStreamWithLogger(
    "glm", null, "glm-5.2", "conn-1", null, null, null, sourceFormat
  );

  const reader = upstream.pipeThrough(transform).getReader();
  const dec = new TextDecoder();
  let out = "";
  while (true) {
    const { value, done } = await reader.read();
    if (done) break;
    out += dec.decode(value, { stream: true });
  }
  return out;
}

function sse(obj) {
  return "event: " + obj.type + "\ndata: " + JSON.stringify(obj) + "\n\n";
}

describe("Claude passthrough EOF safety net (IVT-303)", () => {
  // GLM/z.ai closes mid-tool_use without message_stop → client parser hits
  // "API Error: JSON Parse error: Unexpected EOF" reassembling partial_json.
  it("synthesizes content_block_stop + message_delta + message_stop when upstream omits them", async () => {
    const truncatedClaudeStream = [
      sse({ type: "message_start", message: { id: "msg_1", usage: { input_tokens: 1, output_tokens: 1 } } }),
      sse({ type: "content_block_start", index: 0, content_block: { type: "tool_use", id: "tool_1", name: "screenshot", input: {} } }),
      sse({ type: "content_block_delta", index: 0, delta: { type: "input_json_delta", partial_json: '{"url":"ht' } }),
      // upstream EOF here — no content_block_stop, no message_stop
    ];

    const out = await pipe(truncatedClaudeStream, FORMATS.CLAUDE);

    expect(out).toContain("content_block_start");
    expect(out).toContain("input_json_delta");
    // Synthesized terminal events for the open tool_use block.
    expect(out).toContain("event: content_block_stop");
    expect(out).toMatch(/"index"\s*:\s*0/);
    expect(out).toContain("event: message_delta");
    expect(out).toMatch(/"stop_reason"\s*:\s*"end_turn"/);
    expect(out).toContain("event: message_stop");
    expect(out).toContain("data: [DONE]");
  });

  it("does not double-emit message_stop when upstream already sent one", async () => {
    const completeClaudeStream = [
      sse({ type: "message_start", message: { id: "msg_2", usage: { input_tokens: 1, output_tokens: 1 } } }),
      sse({ type: "content_block_start", index: 0, content_block: { type: "text", text: "" } }),
      sse({ type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "hi" } }),
      sse({ type: "content_block_stop", index: 0 }),
      sse({ type: "message_delta", delta: { stop_reason: "end_turn" }, usage: {} }),
      sse({ type: "message_stop" }),
    ];

    const out = await pipe(completeClaudeStream, FORMATS.CLAUDE);
    const stopCount = (out.match(/event: message_stop/g) || []).length;
    expect(stopCount).toBe(1);
  });

  it("does not synthesize Claude events for non-Claude passthrough", async () => {
    const openaiStream = [
      "data: " + JSON.stringify({ choices: [{ delta: { content: "hi" } }] }) + "\n\n",
      "data: [DONE]\n\n",
    ];

    const out = await pipe(openaiStream, FORMATS.OPENAI);
    expect(out).not.toContain("message_stop");
    expect(out).toContain("data: [DONE]");
  });
});
