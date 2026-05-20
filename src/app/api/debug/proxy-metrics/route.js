import { getProxyMetrics } from "../../../../../open-sse/utils/proxyFetch.js";
import { getStreamMetrics } from "../../../../../open-sse/utils/streamHandler.js";
import { getShutdownStatus } from "../../../../../src/lib/shutdown.js";

/**
 * Debug endpoint exposing current proxy / dispatcher / DNS cache metrics.
 * Useful for load testing and leak validation (P2-01 / P2-02).
 *
 * Enable with ALLOW_DEBUG_ENDPOINTS=1 (or always available in non-production).
 * Never expose in public production without additional auth.
 */
export async function GET() {
  const allow = process.env.ALLOW_DEBUG_ENDPOINTS === "1" || process.env.NODE_ENV !== "production";

  if (!allow) {
    return new Response("Not Found", { status: 404 });
  }

  try {
    const proxy = getProxyMetrics();
    const stream = getStreamMetrics();
    const shutdown = getShutdownStatus();
    return Response.json({ ...proxy, ...stream, shutdown });
  } catch (err) {
    return Response.json({ error: "Failed to collect metrics", message: err?.message }, { status: 500 });
  }
}
