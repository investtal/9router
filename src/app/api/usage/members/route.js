import { NextResponse } from "next/server";
import { getMemberStats } from "@/lib/usageDb";

export const dynamic = "force-dynamic";

const VALID_PERIODS = new Set(["today", "24h", "7d", "30d", "60d", "all"]);
const COLUMNS = [
  "id", "keyName", "apiKeyMasked", "model", "provider",
  "requests", "promptTokens", "completionTokens", "cachedTokens", "cost",
  "meanTPS", "p50TPS", "p95TPS", "throughputTPS", "lastUsed",
];

function windowFor(period) {
  const end = new Date();
  let start;
  if (period === "today") { start = new Date(); start.setHours(0, 0, 0, 0); }
  else if (period === "24h") start = new Date(Date.now() - 86400000);
  else if (period === "7d") start = new Date(Date.now() - 7 * 86400000);
  else if (period === "30d") start = new Date(Date.now() - 30 * 86400000);
  else if (period === "60d") start = new Date(Date.now() - 60 * 86400000);
  else start = null;
  return { start: start ? start.toISOString() : null, end: end.toISOString() };
}

function stripRawKey(cell) {
  const { apiKey, ...rest } = cell;
  return rest;
}

function toCsv(rows) {
  const escape = (v) => {
    if (v === null || v === undefined) return "";
    const s = String(v);
    return /[",\n]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  };
  const lines = [COLUMNS.join(",")];
  for (const r of rows) lines.push(COLUMNS.map((c) => escape(r[c])).join(","));
  return lines.join("\n");
}

export async function GET(request) {
  try {
    const { searchParams } = new URL(request.url);
    const period = searchParams.get("period") || "7d";
    if (!VALID_PERIODS.has(period)) {
      return NextResponse.json({ error: "Invalid period" }, { status: 400 });
    }
    const model = searchParams.get("model");
    const apiKey = searchParams.get("apiKey");
    const format = searchParams.get("format") === "csv" ? "csv" : "json";

    let rows = await getMemberStats(period);
    if (model) rows = rows.filter((r) => r.model === model);
    if (apiKey) rows = rows.filter((r) => r.apiKey === apiKey);
    const members = rows.map(stripRawKey);

    if (format === "csv") {
      const csv = toCsv(members);
      return new NextResponse(csv, {
        status: 200,
        headers: {
          "Content-Type": "text/csv",
          "Content-Disposition": `attachment; filename="members-${period}.csv"`,
        },
      });
    }
    return NextResponse.json({ period, window: windowFor(period), members });
  } catch (error) {
    console.error("[API] /api/usage/members failed:", error);
    return NextResponse.json({ error: "Failed to fetch member stats" }, { status: 500 });
  }
}
