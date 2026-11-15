export type NoteDocumentTabTarget = { readonly kind: "page"; readonly pageId: string }
  | { readonly kind: "writing"; readonly manuscriptId: string; readonly path: string };

export interface OpenNoteDocumentTabRequest {
  readonly target: NoteDocumentTabTarget;
  readonly onOpened: () => void;
  readonly reject: (reason: unknown) => void;
}
export const OPEN_NOTE_DOCUMENT_TAB = "research-assistant:open-note-document-tab";

/** All auxiliary note windows use this contract; storage routing stays in the app host. */
export function openNoteDocumentTab(target: NoteDocumentTabTarget, onOpened: () => void = () => {}): Promise<void> {
  return new Promise((resolve, reject) => {
    const event = new CustomEvent<OpenNoteDocumentTabRequest>(OPEN_NOTE_DOCUMENT_TAB, { cancelable: true,
      detail: { target, onOpened: () => { onOpened(); resolve(); }, reject } });
    if (window.dispatchEvent(event)) reject(new Error("当前界面未接入笔记标签页"));
  });
}
