import { NextResponse } from "next/server";
import { getRequestDetailsForExport } from "@/lib/usageDb";
import { buildBulkRequestsToon } from "@/shared/utils/toonExport";

const PERIODS = new Set(["today", "24h", "7d", "30d", "60d", "all"]);

export async function GET(request) {
  try {
    const { searchParams } = new URL(request.url);
    const periodRaw = (searchParams.get("period") || "7d").toLowerCase();
    const period = PERIODS.has(periodRaw) ? periodRaw : "7d";
    const provider = searchParams.get("provider") || null;
    const limitRaw = parseInt(searchParams.get("limit") || "", 10);
    const limit = Number.isFinite(limitRaw) ? limitRaw : null;
    const format = (searchParams.get("format") || "toon").toLowerCase();

    const result = await getRequestDetailsForExport({ period, provider, limit });

    if (format === "json") {
      const { buildExportableDetail } = await import("@/shared/utils/requestDetailParse");
      const requests = result.details.map((d) => buildExportableDetail(d)).filter(Boolean);
      return NextResponse.json({
        exportedAt: new Date().toISOString(),
        source: "9router",
        period: result.period,
        provider: result.provider,
        scanned: result.scanned,
        total: result.total,
        count: requests.length,
        requests,
      });
    }

    const toon = buildBulkRequestsToon({
      period: result.period,
      provider: result.provider,
      details: result.details,
      scanned: result.scanned,
      total: result.total,
    });

    const stamp = new Date().toISOString().slice(0, 10);
    const filename = `9router-requests-${period}-${stamp}.toon`;
    return new NextResponse(toon, {
      status: 200,
      headers: {
        "Content-Type": "text/plain; charset=utf-8",
        "Content-Disposition": `attachment; filename="${filename}"`,
        "X-Export-Count": String(result.scanned),
        "X-Export-Total": String(result.total),
      },
    });
  } catch (error) {
    console.error("[API] Failed to export request details:", error);
    return NextResponse.json(
      { error: "Failed to export request details" },
      { status: 500 }
    );
  }
}
