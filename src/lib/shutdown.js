/**
 * Lightweight centralized shutdown coordinator (E1 - Phase 3)
 *
 * Goal: Reduce the current fragmentation where many modules independently
 * attach SIGINT/SIGTERM/beforeExit handlers, which leads to races,
 * duplicate cleanup, and missed resource releases (especially DB WAL).
 *
 * Usage:
 *   import { registerShutdownHandler } from '@/lib/shutdown.js';
 *   registerShutdownHandler(async () => { await flushSomething(); }, 'my-feature');
 *
 * Handlers are run in registration order when a shutdown signal arrives.
 * The coordinator guarantees it only runs once.
 */

const handlers = [];
let shuttingDown = false;
let shutdownReason = null;

// Defense-in-depth: if any registered handler hangs, we refuse to stay alive forever.
// Default is 10s. Can be overridden with NINE_ROUTER_SHUTDOWN_TIMEOUT_MS for slower environments.
const DEFAULT_SHUTDOWN_TIMEOUT_MS = 10_000;
const SHUTDOWN_TIMEOUT_MS = Number(process.env.NINE_ROUTER_SHUTDOWN_TIMEOUT_MS) || DEFAULT_SHUTDOWN_TIMEOUT_MS;

function log(msg) {
  try { console.log(`[Shutdown] ${msg}`); } catch {}
}

export function isShuttingDown() {
  return shuttingDown;
}

export function getShutdownReason() {
  return shutdownReason;
}

/**
 * Returns a lightweight snapshot of current shutdown state.
 * Useful for health checks and /debug endpoints.
 */
export function getShutdownStatus() {
  return {
    shuttingDown,
    reason: shutdownReason,
    timestamp: shuttingDown ? Date.now() : null,
  };
}

/**
 * Register a cleanup function to be called during graceful shutdown.
 * @param {Function} fn - async or sync function. Receives (reason) as argument.
 * @param {string} [name] - optional name for logging/diagnostics
 */
export function registerShutdownHandler(fn, name = 'unnamed') {
  if (typeof fn !== 'function') return;
  handlers.push({ fn, name });
}

/**
 * Run all registered handlers (best effort, never throws to caller).
 * Safe to call multiple times — only the first call does work.
 */
export async function runShutdownHandlers(reason = 'unknown') {
  if (shuttingDown) return;
  shuttingDown = true;
  shutdownReason = reason;

  log(`Starting graceful shutdown (reason: ${reason})`);

  const shutdownStart = Date.now();
  const results = [];

  // Safety timeout — prevents the process from hanging forever if a handler is slow or broken.
  const timeoutId = setTimeout(() => {
    const elapsed = Date.now() - shutdownStart;
    console.error(`[Shutdown] WARNING: Graceful shutdown timed out after ${elapsed}ms (hard limit: ${SHUTDOWN_TIMEOUT_MS}ms).`);
    console.error(`[Shutdown] One or more handlers did not complete. Forcing exit to avoid zombie process.`);
    process.exit(1);
  }, SHUTDOWN_TIMEOUT_MS);

  for (const { fn, name } of handlers) {
    const handlerStart = Date.now();
    let status = 'ok';
    let error = null;

    try {
      await Promise.resolve(fn(reason)).catch((err) => {
        status = 'rejected';
        error = err?.message || String(err);
        console.error(`[Shutdown] Handler "${name}" rejected:`, error);
      });
    } catch (err) {
      status = 'threw';
      error = err?.message || String(err);
      console.error(`[Shutdown] Handler "${name}" threw synchronously:`, error);
    }

    const duration = Date.now() - handlerStart;
    results.push({ name, duration, status, error });

    if (duration > 300) {
      log(`Handler "${name}" took ${duration}ms`);
    }
  }

  clearTimeout(timeoutId);

  const totalDuration = Date.now() - shutdownStart;

  // Structured shutdown summary
  console.log(`[Shutdown] Completed in ${totalDuration}ms | reason=${reason}`);
  console.log(`[Shutdown] Handlers: ${results.map(r => `${r.name}(${r.duration}ms,${r.status})`).join(' ')}`);

  log('All shutdown handlers completed');
}

// Attach the actual process listeners exactly once.
function attachProcessListeners() {
  const trigger = (reason) => runShutdownHandlers(reason);

  // Use 'once' so we don't stack listeners if this module is imported multiple times
  process.once('SIGINT', () => trigger('SIGINT'));
  process.once('SIGTERM', () => trigger('SIGTERM'));
  process.once('SIGHUP', () => trigger('SIGHUP'));
  process.once('beforeExit', () => trigger('beforeExit'));

  // Best-effort on uncaught errors during normal operation
  process.once('uncaughtException', (err) => {
    console.error('[Shutdown] uncaughtException during shutdown flow:', err);
    trigger('uncaughtException');
  });
}

attachProcessListeners();

export default {
  registerShutdownHandler,
  runShutdownHandlers,
  isShuttingDown,
  getShutdownReason,
  getShutdownStatus,
};
