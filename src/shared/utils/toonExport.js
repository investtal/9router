/**
 * Encode values to TOON for agent-friendly, token-efficient exports.
 * @see https://toonformat.dev/
 */

import { encode } from "@toon-format/toon";
import { buildExportableDetail } from "./requestDetailParse.js";

export function encodeToToon(value) {
  return encode(value, { indentSize: 2 });
}

export function buildSingleRequestToon(detail) {
  const compact = buildExportableDetail(detail);
  return encodeToToon({
    exportedAt: new Date().toISOString(),
    source: "9router",
    format: "toon",
    request: compact,
  });
}

export function buildBulkRequestsToon({ period, provider = null, details = [], scanned = 0, total = 0 }) {
  const requests = (details || [])
    .map((d) => buildExportableDetail(d))
    .filter(Boolean);
  return encodeToToon({
    exportedAt: new Date().toISOString(),
    source: "9router",
    format: "toon",
    period: period || "custom",
    provider: provider || null,
    scanned: scanned || requests.length,
    total: total || requests.length,
    count: requests.length,
    requests,
  });
}

export function downloadTextFile(filename, text, mime = "text/plain;charset=utf-8") {
  const blob = new Blob([text], { type: mime });
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  a.remove();
  URL.revokeObjectURL(url);
}

export function safeFilenamePart(value, max = 48) {
  return String(value || "export")
    .replace(/[^a-zA-Z0-9._-]+/g, "_")
    .slice(0, max);
}
