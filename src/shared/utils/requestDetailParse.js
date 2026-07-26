/**
 * Client-side helpers to present stored request details:
 * - system / harness prompts
 * - conversation messages
 * - declared tools
 * - tool_use / tool_result activity in the turn
 */

function asArray(v) {
  return Array.isArray(v) ? v : [];
}

function safeJson(value) {
  try {
    return JSON.stringify(value, null, 2);
  } catch {
    return String(value);
  }
}

function textFromContent(content) {
  if (content == null) return "";
  if (typeof content === "string") return content;
  if (typeof content === "number" || typeof content === "boolean") return String(content);
  if (!Array.isArray(content)) {
    if (typeof content === "object") {
      if (typeof content.text === "string") return content.text;
      if (typeof content.content === "string") return content.content;
      if (Array.isArray(content.parts)) return textFromContent(content.parts);
    }
    return safeJson(content);
  }
  return content
    .map((part) => {
      if (typeof part === "string") return part;
      if (!part || typeof part !== "object") return "";
      if (typeof part.text === "string") return part.text;
      if (typeof part.content === "string") return part.content;
      if (typeof part.thinking === "string") return `[thinking] ${part.thinking}`;
      if (part.type === "thinking" || part.type === "redacted_thinking") {
        const body = typeof part.thinking === "string" ? part.thinking : safeJson(part);
        return `[thinking] ${body}`;
      }
      if (part.type === "input_text" || part.type === "output_text" || part.type === "text") {
        return typeof part.text === "string" ? part.text : safeJson(part);
      }
      if (part.type === "tool_use" || part.type === "function_call") {
        return `[tool_use ${part.name || part.function?.name || "?"}] ${safeJson(part.input || part.arguments || {})}`;
      }
      if (part.type === "tool_result") {
        const body = typeof part.content === "string" ? part.content : safeJson(part.content ?? "");
        return `[tool_result ${part.tool_use_id || ""}] ${body}`;
      }
      if (Array.isArray(part.parts)) return textFromContent(part.parts);
      return safeJson(part);
    })
    .filter(Boolean)
    .join("\n");
}

function toolCallsText(toolCalls) {
  return asArray(toolCalls)
    .map((tc) => {
      const name = tc?.function?.name || tc?.name || "?";
      const args = tc?.function?.arguments ?? tc?.arguments ?? tc?.input ?? {};
      const id = tc?.id ? ` id=${tc.id}` : "";
      return `[tool_call ${name}${id}] ${typeof args === "string" ? args : safeJson(args)}`;
    })
    .join("\n");
}

export function extractSystemPrompt(request) {
  if (!request || typeof request !== "object") return "";
  if (typeof request.system === "string") return request.system;
  if (Array.isArray(request.system)) return textFromContent(request.system);
  if (typeof request.instructions === "string") return request.instructions;
  if (request.system_instruction || request.systemInstruction) {
    const sys = request.system_instruction || request.systemInstruction;
    if (typeof sys === "string") return sys;
    if (Array.isArray(sys?.parts)) return sys.parts.map((p) => p?.text || "").join("\n");
  }
  // OpenAI-style system messages
  const msgs = asArray(request.messages);
  const systemMsgs = msgs.filter((m) => m?.role === "system" || m?.role === "developer");
  if (systemMsgs.length) return systemMsgs.map((m) => textFromContent(m.content)).join("\n\n---\n\n");
  return "";
}

function normalizeMessage(m, i) {
  if (m == null) {
    return { index: i, role: "unknown", content: "", preview: "", raw: m };
  }
  if (typeof m === "string") {
    return { index: i, role: "input", content: m, preview: m.slice(0, 160), raw: m };
  }
  const role = m?.role || m?.type || "unknown";
  const parts = [];
  const body = textFromContent(m?.content ?? m?.parts ?? m?.text);
  if (body) parts.push(body);
  if (Array.isArray(m?.tool_calls) && m.tool_calls.length) {
    parts.push(toolCallsText(m.tool_calls));
  }
  // OpenAI Responses / agent turns sometimes park payload on type-only items
  if (!parts.length && (m?.type === "function_call" || m?.type === "tool_use")) {
    parts.push(
      `[tool_use ${m.name || m.function?.name || "?"}] ${safeJson(m.input || m.arguments || m)}`
    );
  }
  if (!parts.length && (m?.type === "function_call_output" || m?.type === "tool_result")) {
    parts.push(`[tool_result ${m.call_id || m.tool_use_id || ""}] ${textFromContent(m.output ?? m.content ?? m)}`);
  }
  if (!parts.length && m && typeof m === "object" && !m.role && !m.content && !m.parts) {
    parts.push(safeJson(m));
  }
  const content = parts.filter(Boolean).join("\n\n");
  const preview = content.replace(/\s+/g, " ").trim().slice(0, 160);
  return { index: i, role, content, preview, raw: m };
}

export function extractMessages(request) {
  if (!request || typeof request !== "object") return [];
  if (Array.isArray(request.messages)) {
    return request.messages.map((m, i) => normalizeMessage(m, i));
  }
  if (Array.isArray(request.input)) {
    return request.input.map((m, i) => normalizeMessage(m, i));
  }
  if (Array.isArray(request.contents)) {
    return request.contents.map((m, i) =>
      normalizeMessage(
        {
          role: m?.role || "content",
          content: m?.parts || m?.content || m,
        },
        i
      )
    );
  }
  return [];
}

/**
 * Compact, agent-friendly shape for a single stored request detail (no raw multi-MB dumps).
 */
export function buildExportableDetail(detail) {
  if (!detail || typeof detail !== "object") return null;
  const request = pickPayload(detail.request, detail.providerRequest);
  const response = pickPayload(detail.response, detail.providerResponse);
  const tokens = detail.tokens || {};
  const system = extractSystemPrompt(request);
  const messages = extractMessages(request).map(({ index, role, content }) => ({
    index,
    role,
    content,
    chars: content.length,
  }));
  const tools = extractDeclaredTools(request).map(({ name, description }) => ({
    name,
    description: description || undefined,
  }));
  const { summary: toolActivity } = extractToolActivity(request, response);
  return {
    id: detail.id || null,
    timestamp: detail.timestamp || null,
    provider: detail.provider || null,
    model: detail.model || null,
    status: detail.status || null,
    connectionId: detail.connectionId || null,
    latency: detail.latency || null,
    tokens: {
      input: tokens.prompt_tokens || tokens.input_tokens || 0,
      output: tokens.completion_tokens || tokens.output_tokens || 0,
      cached: tokens.cached_tokens || tokens.cache_read_input_tokens || 0,
      cacheCreation: tokens.cache_creation_input_tokens || 0,
    },
    system: system || undefined,
    messages,
    tools: tools.length ? tools : undefined,
    toolActivity: toolActivity.length ? toolActivity : undefined,
  };
}

export function extractDeclaredTools(request) {
  const tools = asArray(request?.tools);
  return tools.map((t, i) => {
    const name = t?.name || t?.function?.name || t?.type || `tool_${i}`;
    const description = t?.description || t?.function?.description || "";
    const schema = t?.input_schema || t?.parameters || t?.function?.parameters || null;
    return { name, description, schema, raw: t };
  });
}

export function extractToolActivity(request, response) {
  const calls = []; // { name, phase: 'call'|'result', id, chars, preview }
  const countByName = {};
  const idToName = {};

  const bump = (name, phase, id, payload) => {
    const resolved = name || (id && idToName[id]) || "unknown";
    if (id && name) idToName[id] = name;
    const text = typeof payload === "string" ? payload : JSON.stringify(payload ?? "");
    calls.push({ name: resolved, phase, id: id || null, chars: text.length, preview: text.slice(0, 500) });
    if (!countByName[resolved]) countByName[resolved] = { calls: 0, results: 0, chars: 0 };
    if (phase === "call") countByName[resolved].calls += 1;
    else countByName[resolved].results += 1;
    countByName[resolved].chars += text.length;
  };

  const scanContentBlocks = (blocks, nameHint) => {
    for (const block of asArray(blocks)) {
      if (!block || typeof block !== "object") continue;
      if (block.type === "tool_use" || block.type === "function_call") {
        const id = block.id || block.call_id;
        const name = block.name || nameHint;
        if (id && name) idToName[id] = name;
        bump(name, "call", id, block.input ?? block.arguments);
      } else if (block.type === "tool_result") {
        const id = block.tool_use_id;
        const name = nameHint || (id && idToName[id]) || "tool_result";
        bump(name, "result", id, block.content);
      }
    }
  };

  const scanMessage = (msg) => {
    if (!msg || typeof msg !== "object") return;
    if (msg.role === "tool" || msg.role === "function") {
      const id = msg.tool_call_id || msg.id;
      const name = msg.name || (id && idToName[id]) || "tool";
      bump(name, "result", id, msg.content);
    }
    if (Array.isArray(msg.tool_calls)) {
      for (const tc of msg.tool_calls) {
        const name = tc?.function?.name || tc?.name;
        const id = tc?.id;
        if (id && name) idToName[id] = name;
        const args = tc?.function?.arguments ?? tc?.arguments ?? tc?.input;
        bump(name, "call", id, args);
      }
    }
    if (Array.isArray(msg.content)) scanContentBlocks(msg.content, msg.name);
  };

  // Pass 1: register call ids so results can resolve names
  for (const m of asArray(request?.messages)) {
    if (Array.isArray(m?.tool_calls)) {
      for (const tc of m.tool_calls) {
        const name = tc?.function?.name || tc?.name;
        const id = tc?.id;
        if (id && name) idToName[id] = name;
      }
    }
    for (const block of asArray(m?.content)) {
      if (block?.type === "tool_use" || block?.type === "function_call") {
        const id = block.id || block.call_id;
        if (id && block.name) idToName[id] = block.name;
      }
    }
  }

  for (const m of asArray(request?.messages)) scanMessage(m);
  for (const m of asArray(request?.input)) scanMessage(m);
  scanContentBlocks(request?.content);

  // Response shapes
  if (response && typeof response === "object") {
    if (Array.isArray(response.content)) scanContentBlocks(response.content);
    if (Array.isArray(response.tool_calls)) {
      for (const tc of response.tool_calls) {
        const name = tc?.function?.name || tc?.name;
        const id = tc?.id;
        if (id && name) idToName[id] = name;
        bump(name, "call", id, tc?.function?.arguments ?? tc?.arguments ?? tc?.input);
      }
    }
    for (const choice of asArray(response.choices)) {
      scanMessage(choice?.message);
    }
  }

  // providerResponse as string that is JSON
  if (typeof response === "string") {
    try {
      const parsed = JSON.parse(response);
      return extractToolActivity(request, parsed);
    } catch {
      /* plain text */
    }
  }

  const summary = Object.entries(countByName)
    .map(([name, s]) => ({ name, ...s }))
    .sort((a, b) => b.chars - a.chars || b.calls - a.calls);

  return { calls, summary };
}

function pickPayload(primary, fallback) {
  if (primary && typeof primary === "object" && !Array.isArray(primary)) {
    const has =
      (Array.isArray(primary.messages) && primary.messages.length) ||
      (Array.isArray(primary.input) && primary.input.length) ||
      (Array.isArray(primary.tools) && primary.tools.length) ||
      (typeof primary.system === "string" && primary.system.length) ||
      (typeof primary.instructions === "string" && primary.instructions.length) ||
      (Array.isArray(primary.content) && primary.content.length) ||
      (Array.isArray(primary.tool_calls) && primary.tool_calls.length) ||
      (typeof primary.content === "string" && primary.content.length);
    if (has) return primary;
  }
  return fallback || primary || {};
}

/**
 * Prefer activity from the stored response + last user/assistant turns to avoid
 * re-counting multi-turn history every request.
 */
export function extractToolActivityForAggregate(request, response) {
  const req = request && typeof request === "object" ? { ...request } : {};
  if (Array.isArray(req.messages) && req.messages.length > 6) {
    req.messages = req.messages.slice(-6);
  }
  if (Array.isArray(req.input) && req.input.length > 6) {
    req.input = req.input.slice(-6);
  }
  return extractToolActivity(req, response);
}

export function aggregateToolStats(details) {
  const byName = {};
  let withActivity = 0;

  for (const d of details || []) {
    const request = pickPayload(d?.request, d?.providerRequest);
    const response = pickPayload(d?.response, d?.providerResponse);
    const { summary } = extractToolActivityForAggregate(request, response);

    if (summary.length) {
      withActivity += 1;
      const seenInRequest = new Set();
      for (const row of summary) {
        if (!byName[row.name]) {
          byName[row.name] = {
            name: row.name,
            calls: 0,
            results: 0,
            chars: 0,
            requestCount: 0,
          };
        }
        byName[row.name].calls += row.calls || 0;
        byName[row.name].results += row.results || 0;
        byName[row.name].chars += row.chars || 0;
        if (!seenInRequest.has(row.name)) {
          seenInRequest.add(row.name);
          byName[row.name].requestCount += 1;
        }
      }
    }

    // Declared tools even when the turn has zero activity
    for (const t of extractDeclaredTools(request)) {
      if (!byName[t.name]) {
        byName[t.name] = {
          name: t.name,
          calls: 0,
          results: 0,
          chars: 0,
          requestCount: 0,
          declaredOnly: true,
        };
      }
      byName[t.name].declaredCount = (byName[t.name].declaredCount || 0) + 1;
    }
  }

  const tools = Object.values(byName)
    .map((t) => ({
      ...t,
      declaredOnly: t.calls === 0 && t.results === 0,
      avgCharsPerCall: t.calls > 0 ? Math.round(t.chars / t.calls) : 0,
    }))
    .sort((a, b) => b.chars - a.chars || b.calls - a.calls || a.name.localeCompare(b.name));

  return {
    tools,
    scanned: (details || []).length,
    withActivity,
  };
}

export function formatBytesish(chars) {
  const n = Number(chars);
  if (!Number.isFinite(n) || n < 0) return "—";
  if (n < 1000) return `${n} ch`;
  if (n < 1_000_000) return `${(n / 1000).toFixed(1)}k ch`;
  return `${(n / 1_000_000).toFixed(2)}M ch`;
}
