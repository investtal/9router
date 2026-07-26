import { NextResponse } from "next/server";
import { getToolAggregateStats } from "@/lib/usageDb";

const VALID_PERIODS = new Set(["today", "24h", "7d", "30d", "60d", "all"]);

/**
 * GET /api/usage/tools?period=7d&provider=claude
 * Aggregate tool call / payload stats from observability request details.
 */
export async function GET(request) {
  try {
    const { searchParams } = new URL(request.url);
    const periodRaw = searchParams.get("period") || "24h";
    const period = VALID_PERIODS.has(periodRaw) ? periodRaw : null;
    if (!period) {
      return NextResponse.json(
        { error: `Invalid period. Use one of: ${[...VALID_PERIODS].join(", ")}` },
        { status: 400 }
      );
    }
    const provider = searchParams.get("provider") || null;
    const limitParam = searchParams.get("limit");
    let limit = null;
    if (limitParam != null && limitParam !== "") {
      const limitRaw = parseInt(limitParam, 10);
      if (!Number.isFinite(limitRaw) || limitRaw < 1 || limitRaw > 200) {
        return NextResponse.json(
          { error: "limit must be an integer between 1 and 200" },
          { status: 400 }
        );
      }
      limit = limitRaw;
    }

    const result = await getToolAggregateStats({ period, provider, limit });
    return NextResponse.json(result);
  } catch (error) {
    console.error("[API] Failed to get tool aggregate stats:", error);
    return NextResponse.json(
      { error: "Failed to fetch tool stats" },
      { status: 500 }
    );
  }
}
