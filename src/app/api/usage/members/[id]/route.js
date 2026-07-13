import { NextResponse } from "next/server";
import { getMemberDetail } from "@/lib/usageDb";

export const dynamic = "force-dynamic";

const VALID_PERIODS = new Set(["today", "24h", "7d", "30d", "60d", "all"]);

export async function GET(request, { params }) {
  try {
    const { searchParams } = new URL(request.url);
    const period = searchParams.get("period") || "7d";
    if (!VALID_PERIODS.has(period)) {
      return NextResponse.json({ error: "Invalid period" }, { status: 400 });
    }
    const { id } = await params;
    if (!id) return NextResponse.json({ error: "Missing member id" }, { status: 400 });

    const detail = await getMemberDetail({ apiKeyId: id, period });
    if (!detail) return NextResponse.json({ error: "Member not found" }, { status: 404 });

    return NextResponse.json({ period, member: detail.member, totals: detail.totals, byModel: detail.byModel });
  } catch (error) {
    console.error("[API] /api/usage/members/[id] failed:", error);
    return NextResponse.json({ error: "Failed to fetch member detail" }, { status: 500 });
  }
}
