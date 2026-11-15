export interface RowSelectionInput {
  readonly anchorId: string | undefined;
  readonly clickedId: string;
  readonly currentIds: ReadonlySet<string>;
  readonly currentPrimaryId: string | undefined;
  readonly extendRange: boolean;
  readonly toggle: boolean;
  readonly visibleIds: readonly string[];
}

export interface RowSelectionResult {
  readonly anchorId: string;
  readonly primaryId: string | undefined;
  readonly selectedIds: ReadonlySet<string>;
}

export function retainRowSelectionIds(
  currentIds: ReadonlySet<string>,
  visibleIds: readonly string[],
): ReadonlySet<string> {
  const visible = new Set(visibleIds);
  return new Set([...currentIds].filter((id) => visible.has(id)));
}

export function updateRowSelection(
  input: RowSelectionInput,
): RowSelectionResult {
  const current = new Set(input.currentIds);
  if (input.extendRange && input.anchorId !== undefined) {
    const anchorIndex = input.visibleIds.indexOf(input.anchorId);
    const clickedIndex = input.visibleIds.indexOf(input.clickedId);
    if (anchorIndex >= 0 && clickedIndex >= 0) {
      const selected = input.toggle ? current : new Set<string>();
      const start = Math.min(anchorIndex, clickedIndex);
      const end = Math.max(anchorIndex, clickedIndex);
      input.visibleIds.slice(start, end + 1).forEach((id) => selected.add(id));
      return {
        anchorId: input.anchorId,
        primaryId: input.clickedId,
        selectedIds: selected,
      };
    }
  }

  if (input.toggle) {
    if (current.has(input.clickedId)) current.delete(input.clickedId);
    else current.add(input.clickedId);
    const primaryId = current.has(input.clickedId)
      ? input.clickedId
      : input.currentPrimaryId === input.clickedId
        ? current.values().next().value
        : input.currentPrimaryId;
    return { anchorId: input.clickedId, primaryId, selectedIds: current };
  }

  return {
    anchorId: input.clickedId,
    primaryId: input.clickedId,
    selectedIds: new Set([input.clickedId]),
  };
}
