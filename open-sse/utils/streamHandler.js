// Stream handler with disconnect detection - shared for all providers
import { STREAM_STALL_TIMEOUT_MS } from "../config/runtimeConfig.js";
import { registerShutdownHandler } from "../../src/lib/shutdown.js";

// Lightweight global counter + Set for active stall timers (P2-02 / E1 graceful shutdown)
let activeStallTimerCount = 0;
const activeStallTimers = new Set();

// Get HH:MM:SS timestamp
function getTimeString() {
  return new Date().toLocaleTimeString("en-US", { hour12: false, hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

/**
 * Create stream controller with abort and disconnect detection
 * @param {object} options
 * @param {function} options.onDisconnect - Callback when client disconnects
 * @param {object} options.log - Logger instance
 * @param {string} options.provider - Provider name
 * @param {string} options.model - Model name
 */
export function createStreamController({ onDisconnect, onError, log, provider, model } = {}) {
  const abortController = new AbortController();
  const startTime = Date.now();
  let disconnected = false;
  let abortTimeout = null;

  // Stall timer (for long-reasoning / slow upstream) — now owned by the controller
  let stallTimer = null;

  const clearStall = () => {
    if (stallTimer) {
      activeStallTimers.delete(stallTimer);
      clearTimeout(stallTimer);
      stallTimer = null;
      activeStallTimerCount = Math.max(0, activeStallTimerCount - 1);
    }
  };

  const armStall = () => {
    clearStall();
    stallTimer = setTimeout(() => {
      activeStallTimers.delete(stallTimer);
      stallTimer = null;
      activeStallTimerCount = Math.max(0, activeStallTimerCount - 1);
      // We treat a stall as a non-fatal error that should trigger abort + cleanup
      handleError(new Error("stream stall timeout"));
      abortController.abort();
    }, STREAM_STALL_TIMEOUT_MS);
    activeStallTimerCount++;
    activeStallTimers.add(stallTimer);
  };

  const logStream = (status) => {
    const duration = Date.now() - startTime;
    const p = provider?.toUpperCase() || "UNKNOWN";
    console.log(`[${getTimeString()}] 🌊 [STREAM] ${p} | ${model || "unknown"} | ${duration}ms | ${status}`);
  };

  const handleError = (error) => {
    if (disconnected) return;
    disconnected = true;

    clearStall();

    if (abortTimeout) {
      clearTimeout(abortTimeout);
      abortTimeout = null;
    }

    if (error.name === "AbortError") {
      logStream("aborted");
      return;
    }

    logStream(`error: ${error.message}`);
    onError?.(error);
  };

  return {
    signal: abortController.signal,
    startTime,

    isConnected: () => !disconnected,

    // Public API for stall management (used by pipeWithDisconnect)
    armStall,
    clearStall,

    // Call when client disconnects
    handleDisconnect: (reason = "client_closed") => {
      if (disconnected) return;
      disconnected = true;

      clearStall();

      logStream(`disconnect: ${reason}`);

      // Delay abort to allow cleanup
      abortTimeout = setTimeout(() => {
        abortController.abort();
      }, 500);

      onDisconnect?.({ reason, duration: Date.now() - startTime });
    },

    // Call when stream completes normally
    handleComplete: () => {
      if (disconnected) return;
      disconnected = true;

      clearStall();

      logStream("complete");

      if (abortTimeout) {
        clearTimeout(abortTimeout);
        abortTimeout = null;
      }
    },

    // Call on error (centralized)
    handleError,

    abort: () => {
      clearStall();
      abortController.abort();
    }
  };
}

/**
 * Create transform stream with disconnect detection
 * Wraps existing transform stream and adds abort capability.
 *
 * Stall detection lives in pipeWithDisconnect (tied to upstream byte
 * activity), not here — output of the transform stream may be silent
 * for long periods while raw bytes still flow (e.g. Kiro EventStream
 * binary frames buffering, Claude reasoning streams).
 */
export function createDisconnectAwareStream(transformStream, streamController) {
  const reader = transformStream.readable.getReader();
  const writer = transformStream.writable.getWriter();

  return new ReadableStream({
    async pull(controller) {
      if (!streamController.isConnected()) {
        streamController.clearStall?.();
        controller.close();
        return;
      }

      try {
        const { done, value } = await reader.read();

        if (done) {
          streamController.handleComplete();
          controller.close();
          return;
        }
        controller.enqueue(value);
      } catch (error) {
        streamController.clearStall?.(); // extra safety on downstream errors
        streamController.handleError(error);
        reader.cancel().catch(() => {});
        writer.abort().catch(() => {});
        controller.error(error);
      }
    },

    cancel(reason) {
      streamController.clearStall?.();
      streamController.handleDisconnect(reason || "cancelled");
      reader.cancel().catch(() => {});
      writer.abort().catch(() => {});
    }
  });
}

/**
 * Pipe provider response through transform with disconnect detection.
 *
 * Stall watchdog tracks raw upstream byte activity, not transform output.
 * Reasoning models (Claude thinking via Kiro, etc.) can produce zero SSE
 * output for long stretches while partial EventStream frames keep arriving.
 * Measuring stall on the transform output caused false stalls and the
 * "failed to pipe response" error in Next.
 *
 * Any upstream chunk resets the timer. If no bytes arrive for
 * STREAM_STALL_TIMEOUT_MS, abort the underlying fetch via the controller.
 *
 * @param {Response} providerResponse - Response from provider
 * @param {TransformStream} transformStream - Transform stream for SSE
 * @param {object} streamController - Stream controller from createStreamController
 */
export function pipeWithDisconnect(providerResponse, transformStream, streamController) {
  // We now delegate stall management to the controller (stronger ownership)
  const wrappedController = {
    signal: streamController.signal,
    startTime: streamController.startTime,
    isConnected: () => streamController.isConnected(),
    handleComplete: () => streamController.handleComplete(),
    handleError: (e) => streamController.handleError(e),
    handleDisconnect: (r) => streamController.handleDisconnect(r),
    abort: () => streamController.abort(),
    // Expose the controller's stall helpers so the upstream tap can use them
    armStall: () => streamController.armStall?.(),
    clearStall: () => streamController.clearStall?.()
  };

  try {
    // Arm the stall timer (now owned by the controller)
    streamController.armStall?.();

    const upstreamTap = new TransformStream({
      transform(chunk, controller) {
        streamController.armStall?.();
        controller.enqueue(chunk);
      },
      flush() {
        streamController.clearStall?.();
      }
    });

    const transformedBody = providerResponse.body
      .pipeThrough(upstreamTap)
      .pipeThrough(transformStream);

    return createDisconnectAwareStream(
      { readable: transformedBody, writable: { getWriter: () => ({ abort: () => Promise.resolve() }) } },
      wrappedController
    );
  } catch (err) {
    // Any synchronous error during setup → make sure the controller cleans its timers
    streamController.clearStall?.();
    // Also cancel the original provider body so we don't leave the upstream connection hanging
    providerResponse?.body?.cancel?.().catch(() => {});
    throw err;
  }
}

/**
 * Observability helpers (added in light D)
 */
export function getActiveStallTimerCount() {
  return activeStallTimerCount;
}

export function getStreamMetrics() {
  return {
    activeStallTimers: activeStallTimerCount,
  };
}

/**
 * Graceful shutdown helper (E1).
 * Forcibly clears any remaining stall timers so they don't keep the process
 * or hold references after the main shutdown path has run.
 */
export function closeAllStallTimers() {
  for (const timer of activeStallTimers) {
    try { clearTimeout(timer); } catch {}
  }
  activeStallTimers.clear();
  activeStallTimerCount = 0;
}

// Register stall timer cleanup with the central graceful shutdown coordinator (E1)
registerShutdownHandler(closeAllStallTimers, "stall-timers");

