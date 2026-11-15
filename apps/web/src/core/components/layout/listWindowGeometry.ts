/** Shared geometry for retaining only rows near a scroll viewport. */
export function listWindowOffsets(ids: readonly string[], sizes: ReadonlyMap<string, number>, estimate: number): number[] {
  const offsets = [0];
  for (const id of ids) offsets.push(offsets[offsets.length - 1]! + (sizes.get(id) ?? estimate));
  return offsets;
}

export function listWindowBoundary(offsets: readonly number[], y: number, after: boolean): number {
  let low = 0, high = offsets.length - 1;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (offsets[middle]! < y || (after && offsets[middle] === y)) low = middle + 1;
    else high = middle;
  }
  return low;
}

export function listWindowRange(offsets: readonly number[], top: number, height: number): {first: number; end: number} {
  const count = offsets.length - 1;
  const firstVisible = Math.max(0, listWindowBoundary(offsets, top, true) - 1);
  const endVisible = Math.min(count, listWindowBoundary(offsets, top + height, false));
  const buffer = Math.ceil(Math.max(1, endVisible - firstVisible) / 2);
  return {first: Math.max(0, firstVisible - buffer), end: Math.min(count, Math.max(firstVisible + 1, endVisible) + buffer)};
}
