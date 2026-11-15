import { createNotePdfSnapshot } from "./notePdfSnapshot";
/** Both formats freeze the same production DOM before any PDF conversion. */
export function captureNoteExport(pageId: string, fallbackTitle: string): {
    snapshot: HTMLElement;
    title: string;
} {
    const source = [...document.querySelectorAll<HTMLElement>("[data-note-pdf-page-id]")]
        .find(node => node.dataset.notePdfPageId === pageId && !node.closest("[hidden], [inert]"));
    if (!source)
        throw new Error("NOTE_EXPORT_EDITOR_UNAVAILABLE：笔记编辑器尚未就绪，请打开笔记后重试。");
    const title = source.querySelector('[data-content-type="documentTitle"]')?.textContent?.trim() || fallbackTitle || "无标题";
    return { snapshot: createNotePdfSnapshot(source, title), title };
}
