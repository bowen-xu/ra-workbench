import { isDependentReadingNote, type NotePageId } from "@research-assistant/core/notes";
import { NotesGatewayError, type NotesGateway, type NoteDocumentSnapshot } from "@research-assistant/data/notes";
import { readState, writeState, readingNotesFromState, type BrowserNotesState } from "./browserNotesStorage";
import type { BrowserDocumentSave } from "./browserPageEntryLifecycle";

/** Browser equivalent of the native atomic navigation transaction. */
export function browserNoteNavigation(ports: {
  readonly saveBrowserDocument: BrowserDocumentSave;
  readonly snapshot: (state: BrowserNotesState, pageId: NotePageId) => NoteDocumentSnapshot;
}): Pick<NotesGateway, "commitNavigation" | "listFavoritePageIds" | "reorderFavorites"> {
  const { saveBrowserDocument, snapshot } = ports;
  return {
  async commitNavigation(request) {
    let state = await readState();
    const previous = state.favoriteOrder ?? [];
    if (JSON.stringify(previous) !== JSON.stringify(request.expectedFavoriteIds) || new Set(request.favoriteIds).size !== request.favoriteIds.length
      || request.documents.length > 2 || new Set(request.documents.map(save => save.document.pageId)).size !== request.documents.length)
      throw new NotesGatewayError("invalid_input", "笔记导航已变化，请刷新后重试。");
    const changed = new Set<NotePageId>();
    for (const save of request.documents) {
      if (snapshot(state, save.document.pageId).page.status !== "active") throw new NotesGatewayError("invalid_input", "请先恢复笔记，再移动页面链接。");
      // This transaction transfers an entrance between documents/favorites.
      // An intermediate source removal is not deletion of the target page.
      state = saveBrowserDocument(state, save, "prepared"); changed.add(save.document.pageId);
    }
    const desired = new Set(request.favoriteIds);
    for (const id of new Set([...previous, ...request.favoriteIds])) {
      if (previous.includes(id) === desired.has(id)) continue;
      const page = snapshot(state, id).page;
      if (desired.has(id) && (page.status !== "active" || readingNotesFromState(state).some(note => note.pageId === id && isDependentReadingNote(note))))
        throw new NotesGatewayError("invalid_input", "只能将活动独立笔记加入收藏。");
      state = { ...state, pages: state.pages.map(value => value.id === id ? { ...page, isFavorite: desired.has(id), revision: page.revision + 1 } : value) }; changed.add(id);
    }
    state = { ...state, favoriteOrder: [...request.favoriteIds] };
    // A single storage publication is the transaction boundary, including quota failures.
    await writeState(state);
    return [...changed].map(id => snapshot(state, id));
  },
  async listFavoritePageIds() { return (await readState({metadataOnly: true})).favoriteOrder ?? []; },
  async reorderFavorites(request) {
    const state = await readState({metadataOnly: true}), current = state.favoriteOrder ?? [];
    if (JSON.stringify(current) !== JSON.stringify(request.expectedPageIds)
      || JSON.stringify([...current].sort()) !== JSON.stringify([...request.pageIds].sort())) throw new NotesGatewayError("invalid_input", "收藏列表已变化，请刷新后重新排序。");
    await writeState({ ...state, favoriteOrder: [...request.pageIds] });
  },
  };
}
