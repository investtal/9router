import { describe, it, expect } from "vitest";
import { sortMemberRows } from "../../src/app/(dashboard)/dashboard/usage/components/membersTable.js";

const rows = [
  { keyName: "a", model: "opus", cost: 1, requests: 10, meanTPS: 50 },
  { keyName: "b", model: "opus", cost: 5, requests: 2, meanTPS: 80 },
  { keyName: "c", model: "haiku", cost: 3, requests: 7, meanTPS: 100 },
];

describe("sortMemberRows", () => {
  it("sorts by cost desc by default", () => {
    const out = sortMemberRows(rows);
    expect(out.map((r) => r.keyName)).toEqual(["b", "c", "a"]);
  });
  it("sorts by requests asc", () => {
    const out = sortMemberRows(rows, "requests", "asc");
    expect(out.map((r) => r.keyName)).toEqual(["b", "c", "a"]);
  });
  it("sorts by meanTPS desc", () => {
    const out = sortMemberRows(rows, "meanTPS", "desc");
    expect(out.map((r) => r.keyName)).toEqual(["c", "b", "a"]);
  });
});
