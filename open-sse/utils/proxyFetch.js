import { Readable } from "stream";
import { MEMORY_CONFIG } from "../config/runtimeConfig.js";
import { registerShutdownHandler } from "../../src/lib/shutdown.js";

const originalFetch = globalThis.fetch;
const proxyDispatchers = new Map();

// DNS cache — use Map to avoid prototype pollution via malformed hostnames
const DNS_CACHE = new Map();
const MITM_BYPASS_HOSTS = [
  "cloudcode-pa.googleapis.com",
  "daily-cloudcode-pa.googleapis.com",
  "api.individual.githubcopilot.com",
  "q.us-east-1.amazonaws.com",
  "codewhisperer.us-east-1.amazonaws.com",
  "api2.cursor.sh",
];
const GOOGLE_DNS_SERVERS = ["8.8.8.8", "8.8.4.4"];
const HTTPS_PORT = 443;
const HTTP_SUCCESS_MIN = 200;
const HTTP_SUCCESS_MAX = 300;

// Lightweight periodic sweeper for DNS_CACHE (P2-01)
const DNS_CACHE_SWEEP_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes
setInterval(() => {
  const now = Date.now();
  let cleaned = 0;
  for (const [key, value] of DNS_CACHE) {
    if (value.expiry && now >= value.expiry) {
      DNS_CACHE.delete(key);
      cleaned++;
    }
  }
  if (cleaned > 0 && process.env.NINE_ROUTER_DEBUG_PROXY) {
    console.log(`[ProxyFetch] DNS_CACHE sweeper removed ${cleaned} expired entries`);
  }
}, DNS_CACHE_SWEEP_INTERVAL_MS).unref?.(); // unref so it doesn't keep the process alive

function normalizeString(value) {
  if (value === undefined || value === null) return "";
  return String(value).trim();
}

/**
 * Resolve real IP using Google DNS (bypass system DNS)
 */
async function resolveRealIP(hostname) {
  const cached = DNS_CACHE.get(hostname);
  if (cached && Date.now() < cached.expiry) return cached.ip;

  try {
    const dns = await import("dns");
    const { promisify } = await import("util");
    const resolver = new dns.Resolver();
    resolver.setServers(GOOGLE_DNS_SERVERS);
    const resolve4 = promisify(resolver.resolve4.bind(resolver));
    const addresses = await resolve4(hostname);
    DNS_CACHE.set(hostname, { ip: addresses[0], expiry: Date.now() + MEMORY_CONFIG.dnsCacheTtlMs });
    if (process.env.NINE_ROUTER_DEBUG_PROXY) {
      console.log(`[ProxyFetch][debug] DNS_CACHE set for ${hostname}, size=${DNS_CACHE.size}`);
    }
    return addresses[0];
  } catch (error) {
    console.warn(`[ProxyFetch] DNS resolve failed for ${hostname}:`, error.message);
    return null;
  }
}

/**
 * Check if request should bypass MITM DNS redirect
 */
function shouldBypassMitmDns(url) {
  try {
    const hostname = new URL(url).hostname;
    return MITM_BYPASS_HOSTS.some(host => hostname.includes(host));
  } catch { return false; }
}

function shouldBypassByNoProxy(targetUrl, noProxyValue) {
  const noProxy = normalizeString(noProxyValue);
  if (!noProxy) return false;

  let hostname;
  try { hostname = new URL(targetUrl).hostname.toLowerCase(); } catch { return false; }
  const patterns = noProxy.split(",").map((p) => p.trim().toLowerCase()).filter(Boolean);

  return patterns.some((pattern) => {
    if (pattern === "*") return true;
    if (pattern.startsWith(".")) return hostname.endsWith(pattern) || hostname === pattern.slice(1);
    return hostname === pattern || hostname.endsWith(`.${pattern}`);
  });
}

/**
 * Get proxy URL from environment
 */
function getEnvProxyUrl(targetUrl) {
  const noProxy = process.env.NO_PROXY || process.env.no_proxy;
  if (shouldBypassByNoProxy(targetUrl, noProxy)) return null;

  let protocol;
  try { protocol = new URL(targetUrl).protocol; } catch { return null; }

  if (protocol === "https:") {
    return process.env.HTTPS_PROXY || process.env.https_proxy ||
      process.env.ALL_PROXY || process.env.all_proxy;
  }

  return process.env.HTTP_PROXY || process.env.http_proxy ||
    process.env.ALL_PROXY || process.env.all_proxy;
}

/**
 * Normalize proxy URL (allow host:port)
 */
function normalizeProxyUrl(proxyUrl) {
  const normalizedInput = normalizeString(proxyUrl);
  if (!normalizedInput) return null;

  try {

    new URL(normalizedInput);
    return normalizedInput;
  } catch {
    // Allow "127.0.0.1:7890" style values
    return `http://${normalizedInput}`;
  }
}

function resolveConnectionProxyUrl(targetUrl, proxyOptions) {
  const enabled = proxyOptions?.enabled === true || proxyOptions?.connectionProxyEnabled === true;
  if (!enabled) return null;

  const proxyUrlRaw = normalizeString(proxyOptions?.url ?? proxyOptions?.connectionProxyUrl);
  if (!proxyUrlRaw) return null;

  const noProxy = normalizeString(proxyOptions?.noProxy ?? proxyOptions?.connectionNoProxy);
  if (noProxy && shouldBypassByNoProxy(targetUrl, noProxy)) return null;

  return normalizeProxyUrl(proxyUrlRaw);
}

/**
 * Create proxy dispatcher lazily (undici-compatible)
 */
async function getDispatcher(proxyUrl) {
  const normalized = normalizeProxyUrl(proxyUrl);
  if (!normalized) return null;

  if (!proxyDispatchers.has(normalized)) {
    // Evict oldest entry if max size reached — properly close the undici agent first
    if (proxyDispatchers.size >= MEMORY_CONFIG.proxyDispatchersMaxSize) {
      const oldestKey = proxyDispatchers.keys().next().value;
      if (oldestKey) {
        const oldAgent = proxyDispatchers.get(oldestKey);
        try {
          if (oldAgent && typeof oldAgent.close === "function") {
            // undici ProxyAgent.close() returns a promise; we fire-and-forget
            // to avoid blocking new dispatcher creation under load.
            Promise.resolve(oldAgent.close()).catch((e) => {
              console.warn(`[ProxyFetch] Failed to close evicted ProxyAgent: ${e?.message || e}`);
            });
          }
        } catch (e) {
          console.warn(`[ProxyFetch] Error during ProxyAgent eviction: ${e?.message || e}`);
        }
        proxyDispatchers.delete(oldestKey);
      }
    }

    const { ProxyAgent } = await import("undici");

    // Improved ProxyAgent options for stability under long-running load (P2-01)
    proxyDispatchers.set(normalized, new ProxyAgent({
      uri: normalized,
      maxSockets: 8,              // Limit concurrent sockets per proxy
      keepAliveTimeout: 30_000,   // 30s idle keep-alive
      connectTimeout: 10_000,
      bodyTimeout: 180_000,       // Allow long tool + reasoning responses
      headersTimeout: 30_000,
      pipelining: 1,              // Conservative
    }));
  }

  const result = proxyDispatchers.get(normalized);

  if (process.env.NINE_ROUTER_DEBUG_PROXY) {
    console.log(`[ProxyFetch][debug] proxyDispatchers.size=${proxyDispatchers.size} DNS_CACHE.size=${DNS_CACHE.size}`);
  }

  return result;
}

/**
 * Create HTTPS request with manual socket connection (bypass DNS)
 * P2-01 hardening: now respects AbortSignal and guarantees resource cleanup.
 */
async function createBypassRequest(parsedUrl, realIP, options = {}) {
  const httpsModule = await import("https");
  const netModule = await import("net");
  // CJS modules expose exports via .default in ESM dynamic import context
  const https = httpsModule.default ?? httpsModule;
  const net = netModule.default ?? netModule;

  return new Promise((resolve, reject) => {
    const socket = new net.Socket();
    let req = null;
    let aborted = false;

    const destroyResources = () => {
      try { socket.destroy(); } catch {}
      try { if (req) req.destroy(); } catch {}
    };

    const signal = options.signal;
    const onAbort = () => {
      if (aborted) return;
      aborted = true;
      destroyResources();
      reject(new DOMException("The operation was aborted", "AbortError"));
    };

    if (signal) {
      if (signal.aborted) {
        onAbort();
        return;
      }
      signal.addEventListener("abort", onAbort, { once: true });
    }

    socket.connect(HTTPS_PORT, realIP, () => {
      if (aborted) return;

      const reqOptions = {
        socket,
        // SNI + cert hostname are validated against the hostname the caller
        // asked for, not the IP we connected to. This keeps the DNS-bypass
        // (avoiding /etc/hosts MITM) while still rejecting on-path attackers
        // that present a different cert. The MITM_BYPASS_HOSTS targets are
        // all public-CA-issued (Google / GitHub / AWS / Cursor) so default
        // verification works without any extra trust store.
        servername: parsedUrl.hostname,
        path: parsedUrl.pathname + parsedUrl.search,
        method: options.method || "POST",
        headers: {
          ...options.headers,
          Host: parsedUrl.hostname,
        },
      };

      req = https.request(reqOptions, (res) => {
        if (aborted) return;
        const response = {
          ok: res.statusCode >= HTTP_SUCCESS_MIN && res.statusCode < HTTP_SUCCESS_MAX,
          status: res.statusCode,
          statusText: res.statusMessage,
          headers: new Map(Object.entries(res.headers)),
          body: Readable.toWeb(res),
          text: async () => {
            const chunks = [];
            for await (const chunk of res) chunks.push(chunk);
            return Buffer.concat(chunks).toString();
          },
          json: async () => JSON.parse(await response.text()),
        };
        resolve(response);
      });

      req.on("error", (err) => {
        if (aborted) return;
        destroyResources();
        reject(err);
      });

      if (options.body) {
        req.write(typeof options.body === "string" ? options.body : JSON.stringify(options.body));
      }
      req.end();
    });

    socket.on("error", (err) => {
      if (aborted) return;
      destroyResources();
      reject(err);
    });
  });
}

/**
 * Returns current proxy-related metrics (useful for load tests and debugging).
 * Only meaningful when debug mode or external monitoring is enabled.
 */
export function getProxyMetrics() {
  return {
    proxyDispatchers: proxyDispatchers.size,
    dnsCache: DNS_CACHE.size,
    timestamp: Date.now(),
  };
}

/**
 * Graceful shutdown helper (E1).
 * Closes every remaining ProxyAgent and clears both caches.
 * This completes the P2-01 work on the shutdown path.
 */
export async function closeAllProxyResources() {
  const closePromises = [];

  for (const [key, agent] of proxyDispatchers) {
    if (agent && typeof agent.close === "function") {
      closePromises.push(
        Promise.resolve(agent.close()).catch((e) =>
          console.warn(`[ProxyFetch] Error closing dispatcher ${key} on shutdown: ${e?.message || e}`)
        )
      );
    }
  }

  await Promise.allSettled(closePromises);

  proxyDispatchers.clear();
  DNS_CACHE.clear();

  if (process.env.NINE_ROUTER_DEBUG_PROXY) {
    console.log("[ProxyFetch] Closed all proxy dispatchers and cleared DNS cache on shutdown");
  }
}

export async function proxyAwareFetch(url, options = {}, proxyOptions = null) {
  const targetUrl = typeof url === "string" ? url : url.toString();

  // Vercel relay: forward request via relay headers
  const vercelRelayUrl = normalizeString(proxyOptions?.vercelRelayUrl);
  if (vercelRelayUrl) {
    const parsed = new URL(targetUrl);
    const relayHeaders = {
      ...options.headers,
      "x-relay-target": `${parsed.protocol}//${parsed.host}`,
      "x-relay-path": `${parsed.pathname}${parsed.search}`,
    };
    return originalFetch(vercelRelayUrl, { ...options, headers: relayHeaders });
  }

  const connectionProxyUrl = resolveConnectionProxyUrl(targetUrl, proxyOptions);
  const envProxyUrl = connectionProxyUrl ? null : normalizeProxyUrl(getEnvProxyUrl(targetUrl));
  const proxyUrl = connectionProxyUrl || envProxyUrl;

  // MITM DNS bypass: for known MITM-intercepted hosts, resolve real IP to avoid DNS spoof
  if (shouldBypassMitmDns(targetUrl)) {
    if (proxyUrl) {
      // Proxy resolves DNS externally (not affected by /etc/hosts) — use proxy directly
      try {
        const dispatcher = await getDispatcher(proxyUrl);
        return await originalFetch(url, { ...options, dispatcher });
      } catch (proxyError) {
        if (proxyOptions?.strictProxy === true) {
          throw new Error(`[ProxyFetch] Proxy required but failed (strictProxy=true): ${proxyError.message}`);
        }
        console.warn(`[ProxyFetch] Proxy failed, falling back to direct bypass: ${proxyError.message}`);
      }
    }
    // No proxy — manually resolve real IP to bypass DNS spoof
    try {
      const parsedUrl = new URL(targetUrl);
      const realIP = await resolveRealIP(parsedUrl.hostname);
      if (realIP) return await createBypassRequest(parsedUrl, realIP, options);
    } catch (error) {
      console.warn(`[ProxyFetch] MITM bypass failed: ${error.message}`);
    }
  }

  if (proxyUrl) {
    try {
      const dispatcher = await getDispatcher(proxyUrl);
      return await originalFetch(url, { ...options, dispatcher });
    } catch (proxyError) {
      // If strictProxy is enabled, fail hard instead of falling back to direct
      if (proxyOptions?.strictProxy === true) {
        throw new Error(`[ProxyFetch] Proxy required but failed (strictProxy=true): ${proxyError.message}`);
      }
      console.warn(`[ProxyFetch] Proxy failed, falling back to direct: ${proxyError.message}`);
      return originalFetch(url, options);
    }
  }

  return originalFetch(url, options);
}

// Register proxy resource cleanup with the central graceful shutdown coordinator (E1)
registerShutdownHandler(closeAllProxyResources, "proxy-resources");

/**
 * Patched global fetch with env-proxy support and MITM DNS bypass
 */
async function patchedFetch(url, options = {}) {
  return proxyAwareFetch(url, options, null);
}

// Idempotency guard — only patch once to avoid wrapping multiple times
if (globalThis.fetch !== patchedFetch) {
  globalThis.fetch = patchedFetch;
}

export default patchedFetch;
