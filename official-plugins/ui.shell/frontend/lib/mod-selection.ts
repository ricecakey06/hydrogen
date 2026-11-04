export function selectionFromBox(original: readonly string[], intersected: Iterable<string>): string[] {
  const next = new Set(original);
  for (const file of new Set(intersected)) {
    if (!next.delete(file)) next.add(file);
  }
  return [...next];
}
