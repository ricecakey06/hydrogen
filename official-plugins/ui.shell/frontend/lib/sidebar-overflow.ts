// Reserve the last row for More whenever some contributions cannot fit.
export function sidebarVisibleCount(count: number, available: number, row: number, gap: number, limit: number) {
  const fits = Math.max(0, Math.floor((available + gap) / (row + gap)));
  if (count <= limit && count <= fits) return count;
  return Math.max(0, Math.min(limit, count, fits - 1));
}
