#!/usr/bin/env node
/**
 * Simple load test harness for detecting FD / memory leaks in 9Router streaming.
 *
 * Focus: Linux (can also run on macOS)
 *
 * Usage:
 *   node scripts/load-test-streams.js --concurrency 30 --duration 300000 --abort-rate 0.4
 *
 * It will spam concurrent chat completion requests (with fake tool calls)
 * and randomly abort some of them to simulate real unstable usage.
 */

import { setTimeout as sleep } from "timers/promises";

const DEFAULT_BASE = "http://localhost:20128";
const DEFAULT_MODEL = "kr/claude-sonnet-4.5"; // or any model you have configured

const args = process.argv.slice(2);
const concurrency = Number(args.find(a => a.startsWith("--concurrency="))?.split("=")[1]) || 25;
const durationMs = Number(args.find(a => a.startsWith("--duration="))?.split("=")[1]) || 5 * 60 * 1000; // 5 minutes default
const abortRate = Number(args.find(a => a.startsWith("--abort-rate="))?.split("=")[1]) || 0.35;

const baseUrl = process.env.NINE_ROUTER_URL || DEFAULT_BASE;
const apiKey = process.env.NINE_ROUTER_API_KEY || "sk_test";

console.log(`[LoadHarness] Starting leak test`);
console.log(`  Base URL     : ${baseUrl}`);
console.log(`  Concurrency  : ${concurrency}`);
console.log(`  Duration     : ${durationMs / 1000}s`);
console.log(`  Abort rate   : ${abortRate}`);
console.log(`  Target model : ${DEFAULT_MODEL}`);
console.log(`  Press Ctrl+C to stop early\n`);

let activeRequests = 0;
let totalStarted = 0;
let totalAborted = 0;
let totalCompleted = 0;

const startTime = Date.now();

/**
 * Get open file descriptors for the current process (Linux-focused).
 * Falls back gracefully on macOS / other platforms.
 */
function getOpenFds() {
  try {
    if (process.platform === "linux") {
      const fs = require("fs");
      return fs.readdirSync(`/proc/${process.pid}/fd`).length;
    }
    // macOS / others — best effort
    return process._getActiveRequests?.()?.length || -1;
  } catch {
    return -1;
  }
}

async function runOneStream(shouldAbort = Math.random() < abortRate) {
  activeRequests++;
  totalStarted++;

  const controller = new AbortController();
  let aborted = false;

  // Random lifetime between 8s and 90s to simulate real tool-call sessions
  const lifetime = 8000 + Math.random() * 82000;

  const timeout = setTimeout(() => {
    if (!aborted) {
      aborted = true;
      controller.abort();
      totalAborted++;
    }
  }, lifetime);

  // Occasionally simulate mid-stream error (better error injection)
  const simulateError = Math.random() < 0.08; // ~8% chance
  let errorAfterChunks = simulateError ? Math.floor(Math.random() * 8) + 2 : Infinity;

  try {
    const res = await fetch(`${baseUrl}/v1/chat/completions`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        Authorization: `Bearer ${apiKey}`,
      },
      body: JSON.stringify({
        model: DEFAULT_MODEL,
        messages: [
          {
            role: "user",
            content: "Analyze the current repository structure in detail. Use the available tools (ls, grep, read_file) to explore the codebase and provide a comprehensive summary."
          }
        ],
        max_tokens: 1200,
        stream: true,
        tools: [
          {
            type: "function",
            function: {
              name: "list_directory",
              description: "List files in a directory",
              parameters: { type: "object", properties: { path: { type: "string" } } }
            }
          },
          {
            type: "function",
            function: {
              name: "grep_code",
              description: "Search for code patterns",
              parameters: { type: "object", properties: { pattern: { type: "string" } } }
            }
          }
        ]
      }),
      signal: controller.signal,
    });

    if (!res.ok) {
      console.warn(`[LoadHarness] Non-OK response: ${res.status}`);
      return;
    }

    const reader = res.body.getReader();
    const decoder = new TextDecoder();
    let chunkCount = 0;

    while (true) {
      const { done, value } = await reader.read();
      if (done) break;

      chunkCount++;
      // Simulate slow consumption (realistic for long reasoning)
      await sleep(15 + Math.random() * 40);

      // Mid-stream error injection
      if (chunkCount >= errorAfterChunks) {
        aborted = true;
        controller.abort();
        totalAborted++;
        break;
      }
    }

    totalCompleted++;
  } catch (err) {
    if (controller.signal.aborted) {
      // expected
    } else {
      console.warn(`[LoadHarness] Stream error: ${err.message}`);
    }
  } finally {
    clearTimeout(timeout);
    activeRequests--;
  }
}

const samples = []; // for HTML report

async function monitor() {
  while (Date.now() - startTime < durationMs) {
    const rss = Math.round(process.memoryUsage().rss / 1024 / 1024);
    const fds = getOpenFds();

    const sample = {
      ts: Date.now(),
      active: activeRequests,
      started: totalStarted,
      completed: totalCompleted,
      aborted: totalAborted,
      rssMB: rss,
      fds,
    };
    samples.push(sample);

    console.log(
      `[${new Date().toISOString().slice(11,19)}] ` +
      `Active=${activeRequests.toString().padStart(3)} ` +
      `Started=${totalStarted} Completed=${totalCompleted} Aborted=${totalAborted} ` +
      `RSS=${rss}MB FDs=${fds}`
    );

    // Continuous HTML live view - overwrite the report every 10s
    if (samples.length % 2 === 0) {
      generateHtmlReport(samples, true); // silent = true for live updates
    }

    await sleep(5000);
  }
}

async function main() {
  const monitorPromise = monitor();

  const workers = [];
  for (let i = 0; i < concurrency; i++) {
    workers.push((async () => {
      while (Date.now() - startTime < durationMs) {
        await runOneStream();
        await sleep(200 + Math.random() * 800); // small gap between streams per worker
      }
    })());
  }

  await Promise.all([...workers, monitorPromise]);

  console.log("\n[LoadHarness] Finished.");
  console.log(`Final stats: Started=${totalStarted}, Completed=${totalCompleted}, Aborted=${totalAborted}`);

  // Generate simple HTML report
  generateHtmlReport(samples);
}

function generateHtmlReport(samples, silent = false) {
  if (!samples.length) return;

  const html = `<!DOCTYPE html>
<html>
<head>
  <meta charset="utf-8">
  <meta http-equiv="refresh" content="5">
  <title>9Router Leak Test Report (Live)</title>
  <style>
    body { font-family: system-ui, sans-serif; margin: 2rem; background: #f8f9fa; }
    .metrics { background: white; padding: 1rem; border: 1px solid #ddd; border-radius: 6px; margin-bottom: 1rem; }
    table { border-collapse: collapse; width: 100%; max-width: 1200px; }
    th, td { border: 1px solid #ccc; padding: 6px 10px; text-align: right; font-size: 14px; }
    th { background: #e9ecef; }
    .good { color: #198754; font-weight: bold; }
    .bad { color: #dc3545; font-weight: bold; }
    h1 { margin-bottom: 0.5rem; }
    .live { color: #0d6efd; }
    .metric-value { font-family: monospace; font-size: 1.1em; }
  </style>
  <script>
    // Proxy metrics are polled from the 9Router instance that ran the test
    const PROXY_METRICS_BASE = ${JSON.stringify(baseUrl)};
    async function updateProxyMetrics() {
      try {
        const res = await fetch(PROXY_METRICS_BASE + '/api/debug/proxy-metrics');
        const data = await res.json();
        const el = document.getElementById('proxy-metrics');
        if (el) {
          el.innerHTML = 'Proxy Dispatchers: <span class="metric-value">' + data.proxyDispatchers + '</span> | DNS Cache: <span class="metric-value">' + data.dnsCache + '</span>';
        }
      } catch(e) {}
    }
    setInterval(updateProxyMetrics, 4000);
    window.onload = updateProxyMetrics;
  </script>
</head>
<body>
  <h1>9Router Stream Leak Test Report <span class="live">(Live - auto-refresh 5s)</span></h1>
  
  <div class="metrics">
    <strong>Live Proxy Metrics:</strong> 
    <span id="proxy-metrics">Loading...</span>
    <small>(from /api/debug/proxy-metrics)</small>
  </div>

  <p>Generated: ${new Date().toISOString()} | Samples: ${samples.length}</p>
  
  <table>
    <tr>
      <th>Time</th>
      <th>Active</th>
      <th>Started</th>
      <th>Completed</th>
      <th>Aborted</th>
      <th>RSS (MB)</th>
      <th>Open FDs</th>
    </tr>
    ${samples.slice(-30).map(s => `
      <tr>
        <td>${new Date(s.ts).toLocaleTimeString()}</td>
        <td>${s.active}</td>
        <td>${s.started}</td>
        <td>${s.completed}</td>
        <td>${s.aborted}</td>
        <td>${s.rssMB}</td>
        <td class="${s.fds > 300 ? 'bad' : 'good'}">${s.fds}</td>
      </tr>
    `).join('')}
  </table>
  <p><small>Last 30 samples shown. FDs in red if &gt; 300 (strong leak signal on Linux). Proxy metrics update live via the debug endpoint.</small></p>
</body>
</html>`;

  const fs = require("fs");
  const reportPath = "leak-test-report.html";
  fs.writeFileSync(reportPath, html);
  if (!silent) {
    console.log(`\n[LoadHarness] HTML report written to ${reportPath} (live view + proxy metrics enabled)`);
  }
}

main().catch(console.error);