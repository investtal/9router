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
// 10 seconds is generous for DB WAL + proxy closes + buffer flushes.
const SHUTDOWN_TIMEOUT_MS = 10_000;

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

  // Safety timeout — prevents the process from hanging forever if a handler is slow or broken.
  const timeoutId = setTimeout(() => {
    const elapsed = Date.now() - shutdownStart;
    console.error(`[Shutdown] WARNING: Graceful shutdown timed out after ${elapsed}ms (hard limit: ${SHUTDOWN_TIMEOUT_MS}ms).`);
    console.error(`[Shutdown] One or more handlers did not complete. Forcing exit to avoid zombie process.`);
    process.exit(1);
  }, SHUTDOWN_TIMEOUT_MS);

  for (const { fn, name } of handlers) {
    try {
      await Promise.resolve(fn(reason)).catch((err) => {
        console.error(`[Shutdown] Handler "${name}" rejected:`, err?.message || err);
      });
    } catch (err) {
      console.error(`[Shutdown] Handler "${name}" threw synchronously:`, err?.message || err);
    }
  }

  clearTimeout(timeoutId);
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
};
