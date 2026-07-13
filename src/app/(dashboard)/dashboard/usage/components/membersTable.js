// Pure sort helper, extracted so it is testable without a DOM.
export function sortMemberRows(rows, sortBy = "cost", sortDir = "desc") {
  const copy = [...rows];
  copy.sort((a, b) => {
    const av = a[sortBy];
    const bv = b[sortBy];
    if (av === null || av === undefined) return bv === null || bv === undefined ? 0 : 1;
    if (bv === null || bv === undefined) return -1;
    if (typeof av === "string" && typeof bv === "string") {
      return sortDir === "asc" ? av.localeCompare(bv) : bv.localeCompare(av);
    }
    if (typeof av !== typeof bv) return 0; // mixed types: leave relative order
    return sortDir === "asc" ? av - bv : bv - av;
  });
  return copy;
}
