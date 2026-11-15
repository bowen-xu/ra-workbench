import type { NotePage, NotePageId } from "@research-assistant/core/notes";

export function descendantPageIds(pages: readonly NotePage[], pageId: NotePageId): ReadonlySet<NotePageId> {
  const descendants = new Set<NotePageId>();
  const queue = [pageId];
  while (queue.length > 0) {
    const parent = queue.shift();
    for (const page of pages) if (page.parentPageId === parent && !descendants.has(page.id)) {
      descendants.add(page.id);
      queue.push(page.id);
    }
  }
  return descendants;
}

export function siblingCount(pages: readonly NotePage[], page: NotePage): number {
  return pages.filter((candidate) => candidate.parentPageId === page.parentPageId).length;
}
