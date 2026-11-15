export interface ReorderRowGeometry { readonly id: string; readonly top: number; readonly height: number }

/** Layout-only displacement. Hosts own ordering, locks and the eventual commit. */
export function reorderRowOffsets(rows: readonly ReorderRowGeometry[], order: readonly string[], options: {
  readonly top?: number;
  readonly gap?: number;
} = {}): ReadonlyMap<string, number> {
  const byId = new Map(rows.map(row => [row.id, row]));
  const offsets = new Map<string, number>();
  let top = options.top ?? rows[0]?.top ?? 0;
  const gaps = new Map(rows.map((row, index) => [row.id, Math.max(0, (rows[index + 1]?.top ?? row.top + row.height) - row.top - row.height)]));
  const gap = options.gap ?? [...gaps.values()].find(value => value > 0) ?? 0;
  for (const id of order) {
    const row = byId.get(id);
    if (!row) continue;
    offsets.set(id, top - row.top);
    top += row.height + gap;
  }
  return offsets;
}

export function reorderedRowIds(ids: readonly string[], source: readonly string[], target: string, placement: "before" | "after"): readonly string[] {
  const selected = new Set(source);
  const moved = ids.filter(id => selected.has(id));
  const remaining = ids.filter(id => !selected.has(id));
  const index = remaining.indexOf(target);
  if (index < 0 || moved.length === 0) return ids;
  const position = index + (placement === "after" ? 1 : 0);
  return [...remaining.slice(0, position), ...moved, ...remaining.slice(position)];
}
