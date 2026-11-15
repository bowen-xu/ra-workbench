import type { NotePageId } from "@research-assistant/core/notes";

export const READING_NOTE_PANEL_TRANSFER = "research-assistant:transfer-reading-note-page";
export interface ReadingNotePanelTransferRequest {
  readonly pageId: NotePageId;
  readonly resolve: (ready: boolean) => void;
}

/** The current floating editor must durably release this page before another host loads it. */
export function requestReadingNotePanelTransfer(pageId: NotePageId): Promise<boolean> {
  return new Promise(resolve => {
    const event = new CustomEvent<ReadingNotePanelTransferRequest>(READING_NOTE_PANEL_TRANSFER, { cancelable: true, detail: { pageId, resolve } });
    window.dispatchEvent(event);
    if (!event.defaultPrevented) resolve(true);
  });
}
