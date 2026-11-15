// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import type { NoteDocumentSnapshot, NotesGateway } from "@research-assistant/data/notes";
import { NoteSaveController } from "./noteSaveController";
import { registerNoteTextSession } from "./noteTextEditorSession";
import { deliverTextReceipt, isTextEditorHeld, registerTextInstallPort } from "./noteTextEditorPorts";

const f = vi.hoisted(() => ({ handlers: new Map<string, (event: { payload: unknown }) => void>(), invoke: vi.fn() }));
vi.mock("../../../runtime/platformEvents", () => ({ listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
  f.handlers.set(name, handler); return () => f.handlers.delete(name);
} }));
vi.mock("../../../runtime/platformCore", () => ({ isDesktopRuntime: () => true, invoke: f.invoke }));
const stops: (() => void)[] = [];
afterEach(() => { stops.splice(0).reverse().forEach(stop => stop()); f.handlers.clear(); vi.clearAllMocks(); });
function snapshot(revision: number, text: string): NoteDocumentSnapshot {
  const pageId = notePageId("page");
  return { page: { id: pageId, schemaVersion: 1, title: "Page", revision, position: 0, status: "active", isFavorite: false,
    projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" }, document: { schemaVersion: 1, pageId, blocks: [{
      id: noteBlockId("block"), type: "paragraph", props: {}, children: [], content: { kind: "inline", items: [{ type: "text", text, styles: [] }] },
    }] } };
}
async function session(load?: () => Promise<NoteDocumentSnapshot>) {
  f.invoke.mockResolvedValue(undefined);
  let current = snapshot(1, "Old"), editor = current.document;
  const saved = snapshot(2, "New"), install = vi.fn((after: NoteDocumentV1) => { editor = after; });
  const prepare = vi.fn((_before: NoteDocumentV1, after: NoteDocumentV1) => () => { install(after); return undefined; });
  const loadDocument = vi.fn(load ?? (async () => saved)), saveDocument = vi.fn();
  const gateway = { loadDocument, saveDocument } as unknown as NotesGateway;
  const onError = vi.fn(), onChanged = vi.fn();
  const controller = new NoteSaveController({ pageId: current.page.id, gateway, initialRevision: 1,
    onStatus: vi.fn(), onSnapshot: value => { current = value; } });
  const begin = vi.spyOn(controller, "beginExternalChange");
  stops.push(() => controller.dispose());
  stops.push(registerTextInstallPort("page", { isInstalling: false, read: () => editor, prepare, prepareStructure: prepare, dispose() {} }));
  const stop = registerNoteTextSession({ controller, gateway, getSnapshot: () => current, onError, onChanged });
  stops.push(stop);
  await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_register_note_text_editor", expect.anything()));
  const sessionId = f.invoke.mock.calls.find(([name]) => name === "agent_register_note_text_editor")?.[1].sessionId;
  const pages = [{ pageId: "page", revision: 2, document: saved.document,
    review: { baseline: current.document, origin: "agent" as const, addToHistory: true } }];
  return { controller, begin, install, prepare, loadDocument, saveDocument, onError, onChanged, pages, saved, stop,
    read: () => current, editor: () => editor,
    type: (text: string) => { editor = snapshot(1, text).document; controller.enqueue(editor); },
    adopt: (value: NoteDocumentSnapshot) => { current = value; editor = value.document; controller.updateRevision(value.page.revision); },
    receipt: () => f.handlers.get("research-note-text-receipt")!({ payload: { pages } }),
    release: () => f.handlers.get("research-note-text-release")!({ payload: { requestId: "original-fence", pages: [] } }),
    flush: async () => {
      f.handlers.get("research-note-text-flush")!({ payload: { requestId: "original-fence", sessionId, pageId: "page" } });
      await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_ack_note_text_editor", { requestId: "original-fence", sessionId, error: null }));
    },
  };
}

it("uses the original active lease for native recovery and deduplicates late event/IPC receipts", async () => {
  const s = await session(); await s.flush();
  expect(isTextEditorHeld("page")).toBe(true);
  s.receipt(); await deliverTextReceipt(s.pages); s.release(); await deliverTextReceipt(s.pages);
  expect(s.begin).toHaveBeenCalledOnce(); expect(s.install).toHaveBeenCalledOnce();
  expect(s.read()).toEqual(s.saved); expect(s.editor()).toEqual(s.saved.document);
  expect(s.prepare).toHaveBeenCalledWith(expect.anything(), s.saved.document, s.pages[0]!.review);
  expect(s.saveDocument).not.toHaveBeenCalled(); expect(s.onError).not.toHaveBeenCalled();
});

it("accepts an unfenced original receipt only over the unchanged saved editor projection", async () => {
  const s = await session(); await s.flush(); s.release();
  await vi.waitFor(() => expect(isTextEditorHeld("page")).toBe(false));
  s.receipt(); await deliverTextReceipt(s.pages);
  expect(s.begin).toHaveBeenCalledTimes(2); expect(s.install).toHaveBeenCalledOnce();
  expect(s.read()).toEqual(s.saved); expect(s.onError).not.toHaveBeenCalled();
});

it("does not obtain another lease or install an old receipt over later unsaved input", async () => {
  const s = await session(); await s.flush(); s.release();
  await vi.waitFor(() => expect(isTextEditorHeld("page")).toBe(false));
  s.type("Later unsaved input"); const later = s.editor();
  s.receipt(); await deliverTextReceipt(s.pages);
  expect(s.begin).toHaveBeenCalledOnce(); expect(s.loadDocument).not.toHaveBeenCalled();
  expect(s.install).not.toHaveBeenCalled(); expect(s.editor()).toEqual(later);
  expect(s.read().page.revision).toBe(1); expect(s.controller.hasUnsavedChanges()).toBe(true);
  expect(s.onError).toHaveBeenCalledWith(expect.stringContaining("当前输入已保留"));
});

it("does not reinstall or roll back a later acknowledged source revision", async () => {
  const s = await session(); const later = snapshot(3, "Later saved input"); s.adopt(later);
  s.receipt(); await deliverTextReceipt(s.pages);
  expect(s.begin).not.toHaveBeenCalled(); expect(s.loadDocument).not.toHaveBeenCalled();
  expect(s.install).not.toHaveBeenCalled(); expect(s.read()).toEqual(later);
  expect(s.onError).not.toHaveBeenCalled();
});

it("serializes concurrent native and IPC receipts through one actual save lease", async () => {
  let finish!: (value: NoteDocumentSnapshot) => void;
  const pending = new Promise<NoteDocumentSnapshot>(resolve => { finish = resolve; });
  const s = await session(() => pending);
  s.receipt(); await vi.waitFor(() => expect(s.loadDocument).toHaveBeenCalledOnce());
  s.receipt(); const ipc = deliverTextReceipt(s.pages);
  expect(s.begin).toHaveBeenCalledOnce(); expect(s.read().page.revision).toBe(1);
  finish(s.saved); await ipc;
  expect(s.begin).toHaveBeenCalledOnce(); expect(s.install).toHaveBeenCalledOnce();
  expect(s.onError).not.toHaveBeenCalled(); expect(s.read()).toEqual(s.saved);
});

it("keeps later input when an original receipt read is already in flight", async () => {
  let finish!: (value: NoteDocumentSnapshot) => void;
  const pending = new Promise<NoteDocumentSnapshot>(resolve => { finish = resolve; });
  const s = await session(() => pending);
  s.receipt(); await vi.waitFor(() => expect(s.loadDocument).toHaveBeenCalledOnce());
  s.type("Input after receipt dispatch"); const later = s.editor();
  finish(s.saved); await deliverTextReceipt(s.pages);
  expect(s.install).not.toHaveBeenCalled(); expect(s.editor()).toEqual(later);
  expect(s.read().page.revision).toBe(1); expect(s.controller.hasUnsavedChanges()).toBe(true);
  expect(s.onError).toHaveBeenCalled();
});

it("does not install a delayed receipt after the original session is disposed", async () => {
  let finish!: (value: NoteDocumentSnapshot) => void;
  const pending = new Promise<NoteDocumentSnapshot>(resolve => { finish = resolve; });
  const s = await session(() => pending);
  s.receipt(); await vi.waitFor(() => expect(s.loadDocument).toHaveBeenCalledOnce());
  s.stop(); s.controller.dispose(); finish(s.saved); await deliverTextReceipt(s.pages);
  expect(s.install).not.toHaveBeenCalled(); expect(s.onChanged).not.toHaveBeenCalled();
  expect(s.read().page.revision).toBe(1);
});
