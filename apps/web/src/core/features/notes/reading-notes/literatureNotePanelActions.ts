import type { NotePageId } from "@research-assistant/core/notes";

export const LITERATURE_NOTE_TRASH_EVENT = "research-assistant:trash-literature-note";
export interface LiteratureNoteTrashRequest {
  readonly pageId: NotePageId;
  readonly resolve: () => void;
  readonly reject: (reason: unknown) => void;
}

/** The app-owned editing host protects any open draft before moving a page to trash. */
export function requestLiteratureNoteTrash(pageId: NotePageId): Promise<void> {
  return new Promise((resolve, reject) => {
    const event = new CustomEvent<LiteratureNoteTrashRequest>(LITERATURE_NOTE_TRASH_EVENT, { cancelable: true, detail: { pageId, resolve, reject } });
    if (window.dispatchEvent(event)) reject(new Error("笔记编辑器尚未就绪，请重试删除。"));
  });
}
