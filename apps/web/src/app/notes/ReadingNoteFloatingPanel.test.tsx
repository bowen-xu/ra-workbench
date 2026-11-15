// @vitest-environment happy-dom
import { act, useImperativeHandle, type Ref } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type ReadingNoteBlock, type NotePageId } from "@research-assistant/core/notes";
import { ReadingNoteFloatingPanel } from "./ReadingNoteFloatingPanel";
import type {WorkspaceClosePort as NoteWorkspacePort} from "../../core/contracts/workbench";
import { clampReadingNoteGeometry, resizeReadingNoteGeometry } from "../../core/features/notes/reading-notes/useReadingNotePanelGeometry";
import { requestLiteratureNoteTrash } from "../../core/features/notes/reading-notes/literatureNotePanelActions";
import { requestReadingNotePanelTransfer } from "../../core/features/notes/reading-notes/readingNotePanelTransfer";
import { OPEN_NOTE_DOCUMENT_TAB, type OpenNoteDocumentTabRequest } from "../../core/features/notes/workspace/openNoteDocumentTab";
import {notify} from "../../core/services/notifications";

const state = vi.hoisted(() => ({ setStatus: vi.fn(), load: vi.fn(), get: vi.fn(), prepare: vi.fn(), migrate: vi.fn(), mounts: [] as string[], unmounts: [] as string[] }));
vi.mock("../../core/services/notifications", () => ({notify:{error:vi.fn()}}));
vi.mock("../../core/features/notes/NotesClient", () => { const gateway = { getReadingNote: state.get, loadDocument: state.load, setPageStatus: state.setStatus }; const client = { gateway, mutate: (operation: (adapter: typeof gateway) => Promise<unknown>) => operation(gateway) }; return { useNotesClient: () => client, useNotesVersion: () => 0 }; });
vi.mock("../../core/features/notes/workspace/noteRecovery", () => ({ migrateReadingNoteRecoveryDraft: state.migrate, hasLegacyReadingRecoveryDraft: () => false }));
vi.mock("../../core/features/notes/workspace/CoreNoteWorkspace", async () => {
  const { useEffect } = await import("react");
  return { CoreNoteWorkspace: ({ initialPageId, ref }: { initialPageId: NotePageId; ref: Ref<NoteWorkspacePort> }) => {
    useImperativeHandle(ref, () => ({ prepareClose: state.prepare }));
    useEffect(() => { state.mounts.push(initialPageId); return () => { state.unmounts.push(initialPageId); }; }, [initialPageId]);
    return <textarea aria-label="共同文档编辑器" defaultValue={initialPageId} />;
  } };
});
let container: HTMLDivElement, root: ReturnType<typeof createRoot>;
function reading(id: string): ReadingNoteBlock {
  return { id: noteBlockId(id), pageId: notePageId(`page-${id}`), noteType: "annotation", body: "summary", color: "#ffcc00", status: "active", isFavorite: false,
    revision: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now",
    source: { paperId: "paper", paperTitle: "paper", attachmentId: "pdf", contentSha256: "a".repeat(64), pageIndex: 0, rects: [], quoteExact: "quote", quotePrefix: "", quoteSuffix: "", state: "exact" } };
}
async function open(id: string) { await act(async () => { window.dispatchEvent(new CustomEvent("research-assistant:open-reading-note", { detail: id })); }); }
beforeEach(async () => {
  vi.resetAllMocks(); state.mounts.length = state.unmounts.length = 0;
  state.get.mockImplementation(async (id: string) => reading(id)); state.prepare.mockResolvedValue(true);
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div"); document.body.append(container); root = createRoot(container);
  await act(async () => root.render(<ReadingNoteFloatingPanel />));
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); vi.unstubAllGlobals(); });
it("expands the same canonical page only after saving and keeps the floating input if navigation is refused",async()=>{
  await open("one");const editor=document.querySelector("textarea")!;editor.value="保留草稿";
  let request: OpenNoteDocumentTabRequest | undefined;
  const listener=(event:Event)=>{ event.preventDefault(); request=(event as CustomEvent<OpenNoteDocumentTabRequest>).detail; };
  window.addEventListener(OPEN_NOTE_DOCUMENT_TAB,listener);
  try{
    state.prepare.mockResolvedValue(false);
    await act(async()=>document.querySelector<HTMLButtonElement>('[aria-label="在笔记标签页中打开"]')!.click());
    expect(request).toBeUndefined();expect(document.querySelector("textarea")).toBe(editor);
    state.prepare.mockResolvedValue(true);
    await act(async()=>document.querySelector<HTMLButtonElement>('[aria-label="在笔记标签页中打开"]')!.click());
    expect(request).toMatchObject({target: {kind: "page", pageId:"page-one"}});
    expect(editor.value).toBe("保留草稿");expect(document.querySelector("textarea")).toBe(editor);
    await act(async()=>request?.onOpened());expect(document.querySelector("textarea")).toBeNull();
  }finally{window.removeEventListener("research-assistant:open-note-page",listener);}
});
it("opens literature notes in the same durable host without a PDF source or duplicated editor", async () => {
  state.load.mockResolvedValue({ page: { id: notePageId("literature-page"), title: "Paper note" } });
  await open("one"); state.prepare.mockResolvedValue(false);
  await act(async () => window.dispatchEvent(new CustomEvent("research-assistant:open-literature-note", { detail: "literature-page" })));
  expect(state.load).not.toHaveBeenCalled(); expect(document.querySelector("textarea")?.value).toBe("page-one");
  state.prepare.mockResolvedValue(true);
  await act(async () => window.dispatchEvent(new CustomEvent("research-assistant:open-literature-note", { detail: "literature-page" })));
  const editor = document.querySelector("textarea")!; editor.value = "draft";
  expect(document.querySelector('[aria-label="文献笔记编辑面板"]')).not.toBeNull();
  await act(async () => window.dispatchEvent(new CustomEvent("research-assistant:open-literature-note", { detail: "literature-page" })));
  expect(document.querySelector("textarea")).toBe(editor); expect(editor.value).toBe("draft");
  await open("two"); expect(document.querySelectorAll("textarea")).toHaveLength(1);
  expect(document.querySelector("textarea")?.value).toBe("page-two");
});
it("protects an open literature draft before trash and closes only after persistence succeeds", async () => {
  const id = notePageId("paper-note");
  state.load.mockResolvedValue({ page: { id, revision: 5, title: "Paper note" } });
  state.setStatus.mockResolvedValue({ id, status: "trashed" });
  await act(async () => window.dispatchEvent(new CustomEvent("research-assistant:open-literature-note", { detail: id })));
  state.prepare.mockResolvedValue(false);
  await act(async () => { await expect(requestLiteratureNoteTrash(id)).rejects.toThrow("草稿已保留"); });
  expect(state.setStatus).not.toHaveBeenCalled(); expect(document.querySelector("textarea")).not.toBeNull();
  state.prepare.mockResolvedValue(true);
  await act(async () => requestLiteratureNoteTrash(id));
  expect(state.prepare).toHaveBeenLastCalledWith(true);
  expect(state.setStatus).toHaveBeenCalledExactlyOnceWith({ id, status: "trashed", expectedRevision: 5 });
  expect(document.querySelector("textarea")).toBeNull();
});
it("opens annotations in the existing host and preserves the same editor when opened again", async () => {
  state.load.mockResolvedValue({page:{id:notePageId("annotation"),title:"AI领域 → 领域"}});
  const show=()=>window.dispatchEvent(new CustomEvent("research-assistant:open-annotation-note",{detail:{pageId:"annotation",kind:"statement"}}));
  await act(async()=>show());
  const editor=document.querySelector("textarea")!;editor.value="语句笔记草稿";
  expect(document.querySelector('[aria-label="语句笔记编辑面板"]')).not.toBeNull();
  await act(async()=>show());
  expect(document.querySelector("textarea")).toBe(editor);
  expect(editor.value).toBe("语句笔记草稿");
  expect(state.mounts).toEqual(["annotation"]);
  state.prepare.mockResolvedValue(false);await open("one");
  expect(document.querySelector("textarea")).toBe(editor);
  state.prepare.mockResolvedValue(true);await open("one");
  expect(document.querySelector('[aria-label="阅读笔记编辑面板"]')).not.toBeNull();
  expect(document.querySelectorAll("textarea")).toHaveLength(1);
});
it("repeated opening retains the same session, input and selection without a backdrop", async () => {
  await open("one");
  const editor = document.querySelector("textarea")!; editor.value = "unfinished"; editor.setSelectionRange(3, 5);
  await open("one"); await act(async () => root.render(<ReadingNoteFloatingPanel />));
  expect(document.querySelector("textarea")).toBe(editor); expect(editor.value).toBe("unfinished"); expect(editor.selectionStart).toBe(3);
  expect(state.mounts).toEqual(["page-one"]); expect(state.prepare).not.toHaveBeenCalled();
  expect(document.querySelector('[aria-modal="false"]')).not.toBeNull(); expect(document.querySelector(".notes-explorer-backdrop")).toBeNull();
});
it("durably releases the selected page before opening it in another host", async () => {
  await open("one");
  const editor = document.querySelector("textarea")!; editor.value = "unfinished";
  state.prepare.mockResolvedValue(false);
  await act(async () => { expect(await requestReadingNotePanelTransfer(notePageId("page-one"))).toBe(false); });
  expect(document.querySelector("textarea")).toBe(editor);
  await act(async () => { expect(await requestReadingNotePanelTransfer(notePageId("another-page"))).toBe(true); });
  expect(state.prepare).toHaveBeenCalledOnce();
  state.prepare.mockResolvedValue(true);
  await act(async () => { expect(await requestReadingNotePanelTransfer(notePageId("page-one"))).toBe(true); });
  expect(document.querySelector("textarea")).toBeNull();
});
it("refuses replacement and closing if neither save nor recovery is durable", async () => {
  await open("one"); state.prepare.mockResolvedValue(false);
  await open("two"); expect(state.get).toHaveBeenCalledOnce(); expect(state.unmounts).toEqual([]);
  await act(async () => document.querySelector<HTMLButtonElement>('[aria-label="关闭阅读笔记"]')?.click());
  expect(document.querySelector("textarea")?.value).toBe("page-one");
  state.prepare.mockResolvedValue(true); await open("two");
  expect(state.unmounts).toEqual(["page-one"]); expect(state.mounts).toEqual(["page-one", "page-two"]);
});
it("serializes replacement and ignores late work after the app owner unmounts", async () => {
  await open("one"); let resolve!: (value: boolean) => void;
  state.prepare.mockImplementationOnce(() => new Promise<boolean>(yes => { resolve = yes; }));
  await open("two"); await open("three"); expect(state.get).toHaveBeenCalledOnce();
  await act(async () => { resolve(true); }); expect(state.mounts).toEqual(["page-one", "page-two"]);
});
it("keeps panel bounds below the app titlebar, including narrow viewports", () => {
  expect(clampReadingNoteGeometry({ x: -30, y: -50, width: 640, height: 560 }, 1280, 808)).toMatchObject({ x: 8, y: 35, width: 640, height: 560 });
  expect(clampReadingNoteGeometry({ x: 100, y: 100, width: 640, height: 560 }, 300, 250)).toEqual({ x: 8, y: 35, width: 284, height: 207 });
});

it("retains both drafts and requires a saved document before opening the old draft", async () => {
  state.migrate.mockReturnValueOnce("collision"); await open("one");
  const editor = document.querySelector("textarea");
  const restore = [...document.querySelectorAll("button")].find(button => button.textContent === "恢复旧阅读草稿")!;
  state.prepare.mockResolvedValue(false); await act(async () => restore.click());
  expect(document.querySelector("textarea")).toBe(editor); expect(state.prepare).toHaveBeenLastCalledWith(true);
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("两份草稿均已保留"));
  state.prepare.mockResolvedValue(true); state.migrate.mockReturnValue("migrated");
  await act(async () => restore.click());
  expect(state.unmounts).toEqual(["page-one"]); expect(state.mounts).toEqual(["page-one", "page-one"]);
});

it("keeps the opposite corner anchored at minimum size and viewport edges", () => {
  const base = { x: 300, y: 150, width: 640, height: 560 };
  expect(resizeReadingNoteGeometry(base, "nw", 1000, 1000, 1280, 808)).toEqual({ x: 620, y: 470, width: 320, height: 240 });
  expect(resizeReadingNoteGeometry(base, "nw", -1000, -1000, 1280, 808)).toEqual({ x: 8, y: 35, width: 932, height: 675 });
  expect(resizeReadingNoteGeometry(base, "se", 1000, 1000, 1280, 808)).toEqual({ x: 300, y: 150, width: 972, height: 650 });
});
it("releases the editor when the current reading document is permanently deleted", async () => {
  await open("one");
  await act(async () => window.dispatchEvent(new CustomEvent("research-assistant:reading-note-deleted", { detail: "one" })));
  expect(document.querySelector("textarea")).toBeNull(); expect(state.unmounts).toEqual(["page-one"]);
});

it("Escape closes the focused reading panel through the same durable save boundary", async () => {
  await open("one"); const editor = document.querySelector("textarea")!; editor.value = "unfinished input";
  const escape = () => editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  state.prepare.mockResolvedValue(false); await act(async () => { escape(); });
  expect(state.prepare).toHaveBeenCalledOnce(); expect(document.querySelector("textarea")).toBe(editor); expect(editor.value).toBe("unfinished input");
  state.prepare.mockResolvedValue(true); await act(async () => { escape(); });
  expect(state.prepare).toHaveBeenCalledTimes(2); expect(document.querySelector('[role="dialog"]')).toBeNull();
});

it("does not close for IME cancellation, handled Escape or background shortcuts", async () => {
  await open("one"); const editor = document.querySelector("textarea")!;
  await act(async () => {
    editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, isComposing: true }));
    const handled = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }); handled.preventDefault(); editor.dispatchEvent(handled);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    const menu = document.createElement("div"); menu.setAttribute("role", "menu"); editor.parentElement!.append(menu);
    menu.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); menu.remove();
    editor.setAttribute("aria-expanded", "true"); editor.setAttribute("aria-controls", "suggestions");
    editor.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    editor.removeAttribute("aria-expanded"); editor.removeAttribute("aria-controls");
  });
  expect(state.prepare).not.toHaveBeenCalled(); expect(document.querySelector("textarea")).toBe(editor);
});
