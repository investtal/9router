
const BASE64_RE = /^(?:data:[^;]+;base64,)?[A-Za-z0-9+/=\s]{500,}$/;
const SECRET_KEY_RE = /^(authorization|proxy-authorization|x-api-key|api[_-]?key|x-goog-api-key|x-auth-token|cookie|set-cookie|token|access[_-]?token|refresh[_-]?token|secret|password|passwd|private[_-]?key|client[_-]?secret|bearer)$/i;
const SECRET_INLINE_RE = /\b(sk-[a-zA-Z0-9_-]{12,}|Bearer\s+[A-Za-z0-9._\-+/=]{12,}|ghp_[A-Za-z0-9]{20,}|xox[baprs]-[A-Za-z0-9-]{10,})\b/g;

function approxSize(value) {
  try {
    return JSON.stringify(value ?? null).length;
  } catch {
    return 0;
  }
}

function redactSecretString(str) {
  if (typeof str !== "string" || str.length < 8) return str;
  return str.replace(SECRET_INLINE_RE, "[redacted-secret]");
}

function redactLargeString(str, maxChars = 8_000) {
  if (typeof str !== "string") return str;
  const scrubbed = redactSecretString(str);
  if (scrubbed.length <= maxChars) return scrubbed;
  if (BASE64_RE.test(scrubbed.slice(0, 600).replace(/\s+/g, ""))) {
    return `[binary/base64 omitted · ${scrubbed.length} chars]`;
  }
  return `${scrubbed.slice(0, maxChars)}\n…[truncated ${scrubbed.length - maxChars} chars]`;
}

function walk(value, depth = 0) {
  if (value == null || depth > 12) return value;
  if (typeof value === "string") return redactLargeString(value);
  if (typeof value !== "object") return value;
  if (Array.isArray(value)) return value.map((v) => walk(v, depth + 1));

  const out = {};
  for (const [k, v] of Object.entries(value)) {
    if (SECRET_KEY_RE.test(k)) {
      out[k] = "[redacted]";
      continue;
    }
    // Drop known huge binary media fields early
    if (
      (k === "data" || k === "image_url" || k === "inline_data" || k === "inlineData") &&
      typeof v === "string" &&
      v.length > 500
    ) {
      out[k] = `[media omitted · ${v.length} chars]`;
      continue;
    }
    if (k === "source" && v && typeof v === "object" && typeof v.data === "string" && v.data.length > 500) {
      out[k] = { ...v, data: `[media omitted · ${v.data.length} chars]` };
      continue;
    }
    if (k === "url" && typeof v === "string" && v.startsWith("data:") && v.length > 500) {
      out[k] = `[media omitted · ${v.length} chars]`;
      continue;
    }
    out[k] = walk(v, depth + 1);
  }
  return out;
}

/**
 * @param {unknown} obj
 * @param {number} maxSize bytes budget for JSON serialization
 * @returns {object}
 */
export function sanitizeForStorage(obj, maxSize = 512 * 1024) {
  if (obj == null) return {};
  if (typeof obj === "string") {
    const text = redactLargeString(obj, Math.min(maxSize, 50_000));
    return { content: text };
  }

  let cleaned;
  try {
    cleaned = walk(typeof structuredClone === "function" ? structuredClone(obj) : JSON.parse(JSON.stringify(obj)));
  } catch {
    cleaned = walk(obj);
  }

  let size = approxSize(cleaned);
  if (size <= maxSize) return cleaned;

  // Prefer dropping oldest conversation turns before system/tools.
  const arrKey = Array.isArray(cleaned.messages)
    ? "messages"
    : Array.isArray(cleaned.input)
      ? "input"
      : Array.isArray(cleaned.contents)
        ? "contents"
        : null;

  if (arrKey && cleaned[arrKey].length > 2) {
    const kept = [...cleaned[arrKey]];
    // Drop from oldest until under budget; re-size less often (every 2 drops)
    let drops = 0;
    while (kept.length > 2) {
      kept.shift();
      drops += 1;
      if (drops % 2 === 0 || kept.length <= 3) {
        size = approxSize({ ...cleaned, [arrKey]: kept });
        if (size <= maxSize) break;
      }
    }
    cleaned = {
      ...cleaned,
      [arrKey]: kept,
      _storageNote: `dropped oldest ${drops} ${arrKey} to fit storage budget`,
    };
    size = approxSize(cleaned);
    if (size <= maxSize) return cleaned;
  }

  // Last resort: hard truncate preview (legacy shape for UI)
  const str = JSON.stringify(cleaned);
  return {
    _truncated: true,
    _originalSize: str.length,
    _preview: str.substring(0, Math.min(4_000, maxSize)),
    system: typeof cleaned.system === "string" ? redactLargeString(cleaned.system, 2_000) : cleaned.system,
    instructions: typeof cleaned.instructions === "string" ? redactLargeString(cleaned.instructions, 2_000) : cleaned.instructions,
    tools: Array.isArray(cleaned.tools)
      ? cleaned.tools.map((t) => ({ name: t?.name || t?.function?.name || "unknown", description: t?.description || t?.function?.description }))
      : cleaned.tools,
    model: cleaned.model,
    stream: cleaned.stream,
  };
}

/** True when a request-shaped object has useful conversation content. */
export function hasRequestPayload(obj) {
  if (!obj || typeof obj !== "object" || Array.isArray(obj)) return false;
  if (obj._truncated) return true;
  if (typeof obj.system === "string" && obj.system.length) return true;
  if (typeof obj.instructions === "string" && obj.instructions.length) return true;
  if (Array.isArray(obj.messages) && obj.messages.length) return true;
  if (Array.isArray(obj.input) && obj.input.length) return true;
  if (Array.isArray(obj.contents) && obj.contents.length) return true;
  if (Array.isArray(obj.tools) && obj.tools.length) return true;
  return false;
}
