import { getAdapter } from "../driver.js";
import { parseJson, stringifyJson } from "../helpers/jsonCol.js";
import { sanitizeForStorage } from "./requestBodySanitize.js";

const DEFAULT_MAX_RECORDS = 200;
const DEFAULT_BATCH_SIZE = 20;
const DEFAULT_FLUSH_INTERVAL_MS = 5000;
// KB unit in settings → bytes here. 2048 KB (~2MB) fits large agent prompts.
const DEFAULT_MAX_JSON_SIZE_KB = 2048;
const DEFAULT_MAX_JSON_SIZE = DEFAULT_MAX_JSON_SIZE_KB * 1024;
const CONFIG_CACHE_TTL_MS = 5000;

let cachedConfig = null;
let cachedConfigTs = 0;

/** Test / settings-update hook — drop cached observability config. */
export function clearObservabilityConfigCache() {
  cachedConfig = null;
  cachedConfigTs = 0;
}

async function getObservabilityConfig() {
  if (cachedConfig && (Date.now() - cachedConfigTs) < CONFIG_CACHE_TTL_MS) return cachedConfig;
  try {
    const { getSettings } = await import("./settingsRepo.js");
    const settings = await getSettings();
    const envEnabled = process.env.OBSERVABILITY_ENABLED !== "false";
    // Profile UI writes enableObservability; older tests used enableObservability2.
    const enabledFlag = settings.enableObservability ?? settings.enableObservability2;
    const enabled = typeof enabledFlag === "boolean" ? enabledFlag : envEnabled;
    const maxJsonSizeKb = settings.observabilityMaxJsonSize
      || parseInt(process.env.OBSERVABILITY_MAX_JSON_SIZE || String(DEFAULT_MAX_JSON_SIZE_KB), 10);
    cachedConfig = {
      enabled,
      maxRecords: settings.observabilityMaxRecords || parseInt(process.env.OBSERVABILITY_MAX_RECORDS || String(DEFAULT_MAX_RECORDS), 10),
      batchSize: settings.observabilityBatchSize || parseInt(process.env.OBSERVABILITY_BATCH_SIZE || String(DEFAULT_BATCH_SIZE), 10),
      flushIntervalMs: settings.observabilityFlushIntervalMs || parseInt(process.env.OBSERVABILITY_FLUSH_INTERVAL_MS || String(DEFAULT_FLUSH_INTERVAL_MS), 10),
      maxJsonSize: Math.max(32, maxJsonSizeKb) * 1024,
    };
  } catch {
    cachedConfig = {
      enabled: false,
      maxRecords: DEFAULT_MAX_RECORDS,
      batchSize: DEFAULT_BATCH_SIZE,
      flushIntervalMs: DEFAULT_FLUSH_INTERVAL_MS,
      maxJsonSize: DEFAULT_MAX_JSON_SIZE,
    };
  }
  cachedConfigTs = Date.now();
  return cachedConfig;
}

let writeBuffer = [];
let flushTimer = null;
let isFlushing = false;

function sanitizeHeaders(headers) {
  if (!headers || typeof headers !== "object") return {};
  const sensitiveKeys = ["authorization", "x-api-key", "cookie", "token", "api-key"];
  const sanitized = { ...headers };
  for (const key of Object.keys(sanitized)) {
    if (sensitiveKeys.some((s) => key.toLowerCase().includes(s))) delete sanitized[key];
  }
  return sanitized;
}

function generateDetailId(model) {
  const timestamp = new Date().toISOString();
  const random = Math.random().toString(36).substring(2, 8);
  const modelPart = model ? model.replace(/[^a-zA-Z0-9-]/g, "-") : "unknown";
  return `${timestamp}-${random}-${modelPart}`;
}

function truncateField(obj, maxSize) {
  return sanitizeForStorage(obj, maxSize);
}

async function flushToDatabase() {
  if (isFlushing) return;
  if (writeBuffer.length === 0) return;
  isFlushing = true;
  try {
    // Drain entire buffer (loop in case more pushed during await)
    while (writeBuffer.length > 0) {
      const items = writeBuffer.splice(0, writeBuffer.length);
      const db = await getAdapter();
      const config = await getObservabilityConfig();

      db.transaction(() => {
        for (const item of items) {
          if (!item.id) item.id = generateDetailId(item.model);
          if (!item.timestamp) item.timestamp = new Date().toISOString();
          if (item.request?.headers) item.request.headers = sanitizeHeaders(item.request.headers);

          const record = {
            id: item.id,
            provider: item.provider || null,
            model: item.model || null,
            connectionId: item.connectionId || null,
            timestamp: item.timestamp,
            status: item.status || null,
            latency: item.latency || {},
            tokens: item.tokens || {},
            request: truncateField(item.request, config.maxJsonSize),
            providerRequest: truncateField(item.providerRequest, config.maxJsonSize),
            providerResponse: truncateField(item.providerResponse, config.maxJsonSize),
            response: truncateField(item.response, config.maxJsonSize),
            pxpipe: item.pxpipe || undefined,
          };

          db.run(
            `INSERT INTO requestDetails(id, timestamp, provider, model, connectionId, status, data) VALUES(?, ?, ?, ?, ?, ?, ?) ON CONFLICT(id) DO UPDATE SET timestamp = excluded.timestamp, provider = excluded.provider, model = excluded.model, connectionId = excluded.connectionId, status = excluded.status, data = excluded.data`,
            [record.id, record.timestamp, record.provider, record.model, record.connectionId, record.status, stringifyJson(record)]
          );
        }

        const cnt = db.get(`SELECT COUNT(*) as c FROM requestDetails`);
        if (cnt && cnt.c > config.maxRecords) {
          db.run(
            `DELETE FROM requestDetails WHERE id IN (SELECT id FROM requestDetails ORDER BY timestamp ASC LIMIT ?)`,
            [cnt.c - config.maxRecords]
          );
        }
      });
    }
  } catch (e) {
    console.error("[requestDetailsRepo] Batch write failed:", e);
  } finally {
    isFlushing = false;
  }
}

export async function saveRequestDetail(detail) {
  const config = await getObservabilityConfig();
  if (!config.enabled) return;

  writeBuffer.push(detail);

  // Hard cap on in-memory buffer to prevent unbounded growth under high load or flush failures.
  // This is a safety net; the DB table itself is already capped.
  const MAX_IN_MEMORY_BUFFER = 2000;
  if (writeBuffer.length > MAX_IN_MEMORY_BUFFER) {
    writeBuffer.shift(); // Drop oldest un-flushed detail
  }

  // Trigger immediate flush if batch threshold reached.
  if (writeBuffer.length >= config.batchSize) {
    if (flushTimer) { clearTimeout(flushTimer); flushTimer = null; }
    flushToDatabase().catch((e) => console.error("[requestDetailsRepo] flush err:", e));
  } else if (!flushTimer) {
    flushTimer = setTimeout(() => {
      flushTimer = null;
      flushToDatabase().catch(() => {});
    }, config.flushIntervalMs);
  }
}

export async function getRequestDetails(filter = {}) {
  const db = await getAdapter();
  const conds = [];
  const params = [];

  if (filter.provider) { conds.push("provider = ?"); params.push(filter.provider); }
  if (filter.model) { conds.push("model = ?"); params.push(filter.model); }
  if (filter.connectionId) { conds.push("connectionId = ?"); params.push(filter.connectionId); }
  if (filter.status) { conds.push("status = ?"); params.push(filter.status); }
  if (filter.startDate) { conds.push("timestamp >= ?"); params.push(new Date(filter.startDate).toISOString()); }
  if (filter.endDate) { conds.push("timestamp <= ?"); params.push(new Date(filter.endDate).toISOString()); }

  const where = conds.length ? `WHERE ${conds.join(" AND ")}` : "";
  const cntRow = db.get(`SELECT COUNT(*) as c FROM requestDetails ${where}`, params);
  const totalItems = cntRow ? cntRow.c : 0;

  const page = filter.page || 1;
  const pageSize = filter.pageSize || 50;
  const totalPages = Math.ceil(totalItems / pageSize);
  const offset = (page - 1) * pageSize;

  const rows = db.all(
    `SELECT data FROM requestDetails ${where} ORDER BY timestamp DESC LIMIT ? OFFSET ?`,
    [...params, pageSize, offset]
  );
  // List returns metadata + token/latency only — full bodies via getRequestDetailById.
  const includeBodies = filter.includeBodies === true;
  const details = rows.map((r) => {
    const full = parseJson(r.data, {});
    if (includeBodies) return full;
    if (!full || typeof full !== "object" || Object.keys(full).length === 0) return {};
    return {
      id: full.id,
      timestamp: full.timestamp,
      provider: full.provider,
      model: full.model,
      connectionId: full.connectionId,
      status: full.status,
      latency: full.latency || {},
      tokens: full.tokens || {},
      pxpipe: full.pxpipe,
      _bodyOmitted: true,
    };
  });

  return {
    details,
    pagination: { page, pageSize, totalItems, totalPages, hasNext: page < totalPages, hasPrev: page > 1 },
  };
}

export async function getDistinctProviders() {
  const db = await getAdapter();
  const rows = db.all(`SELECT DISTINCT provider FROM requestDetails WHERE provider IS NOT NULL ORDER BY provider ASC`);
  return rows.map((r) => r.provider);
}

export async function getRequestDetailById(id) {
  const db = await getAdapter();
  const row = db.get(`SELECT data FROM requestDetails WHERE id = ?`, [id]);
  return row ? parseJson(row.data, null) : null;
}

function periodToStartDate(period) {
  const now = Date.now();
  switch (period) {
    case "today": {
      const d = new Date();
      d.setHours(0, 0, 0, 0);
      return d.toISOString();
    }
    case "24h":
      return new Date(now - 24 * 3600_000).toISOString();
    case "7d":
      return new Date(now - 7 * 24 * 3600_000).toISOString();
    case "30d":
      return new Date(now - 30 * 24 * 3600_000).toISOString();
    case "60d":
      return new Date(now - 60 * 24 * 3600_000).toISOString();
    case "all":
      return null;
    default:
      return new Date(now - 24 * 3600_000).toISOString();
  }
}

/**
 * Aggregate tool_use / tool_result stats from stored request details.
 * Scans up to `limit` newest rows in the period (default = observability maxRecords).
 */
export async function getToolAggregateStats({ period = "24h", provider = null, limit = null } = {}) {
  const db = await getAdapter();
  const config = await getObservabilityConfig();
  // Cap scan tightly: each row can hold multi-MB bodies after observability capture.
  const defaultCap = Math.min(config.maxRecords || 200, 200);
  const cap = Math.min(Math.max(limit || defaultCap, 1), 200);

  const conds = [];
  const params = [];
  const startDate = periodToStartDate(period);
  if (startDate) {
    conds.push("timestamp >= ?");
    params.push(startDate);
  }
  if (provider) {
    conds.push("provider = ?");
    params.push(provider);
  }
  const where = conds.length ? `WHERE ${conds.join(" AND ")}` : "";

  const rows = db.all(
    `SELECT data FROM requestDetails ${where} ORDER BY timestamp DESC LIMIT ?`,
    [...params, cap]
  );
  const details = rows.map((r) => parseJson(r.data, {}));

  // Dynamic import keeps parse util out of early boot paths that don't need it
  const { aggregateToolStats } = await import("@/shared/utils/requestDetailParse.js");
  const agg = aggregateToolStats(details);

  return {
    period,
    provider: provider || null,
    scanned: agg.scanned,
    withActivity: agg.withActivity,
    tools: agg.tools,
    limit: cap,
  };
}

const _shutdownHandler = async () => {
  if (flushTimer) { clearTimeout(flushTimer); flushTimer = null; }
  if (writeBuffer.length > 0) await flushToDatabase();
};

function ensureShutdownHandler() {
  process.off("beforeExit", _shutdownHandler);
  process.off("SIGINT", _shutdownHandler);
  process.off("SIGTERM", _shutdownHandler);
  process.off("exit", _shutdownHandler);

  process.on("beforeExit", _shutdownHandler);
  process.on("SIGINT", _shutdownHandler);
  process.on("SIGTERM", _shutdownHandler);
  process.on("exit", _shutdownHandler);
}

ensureShutdownHandler();
