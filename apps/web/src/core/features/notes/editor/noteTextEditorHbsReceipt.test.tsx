// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import type { NoteDocumentSnapshot, NotesGateway } from "@research-assistant/data/notes";
import type { AppHbsDocumentGateway } from "../../../services/hbsDocumentGateway";
import { AppNoteHbsSession } from "../workspace/appNoteHbsSession";
import { registerNoteTextSession } from "./noteTextEditorSession";
import { deliverTextReceipt, isTextEditorHeld, registerTextInstallPort } from "./noteTextEditorPorts";

const io = vi.hoisted(() => ({ handlers: new Map<string, (event: { payload: unknown }) => void>(), invoke: vi.fn() }));
vi.mock("../../../runtime/platformEvents", () => ({ listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
  io.handlers.set(name, handler); return () => io.handlers.delete(name);
} }));
vi.mock("../../../runtime/platformCore", () => ({ isDesktopRuntime: () => true, invoke: io.invoke }));
const stops: (() => void)[] = [];
afterEach(() => { stops.splice(0).reverse().forEach(stop => stop()); io.handlers.clear(); vi.clearAllMocks(); });
function document(text: string): NoteDocumentV1 {
  return { schemaVersion: 1, pageId: notePageId("native-hbs-receipt"), blocks: [{ id: noteBlockId("paragraph"),
    type: "paragraph", props: {}, children: [], content: { kind: "inline", items: [{ type: "text", text, styles: [] }] } }] };
}
async function session() {
  io.invoke.mockResolvedValue(undefined);
  const old = document("Old"), next = document("New"), pageId = old.pageId;
  let authority = { version: 1, saved: old, baseline: old }, preservation: unknown = null;
  let visible = old;
  const loaded: NoteDocumentSnapshot = { page: { id: pageId, schemaVersion: 1, title: "Note", revision: 1, position: 0,
    status: "active", isFavorite: false, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" }, document: old };
  let current = loaded;
  const hbs: AppHbsDocumentGateway = {
    read: vi.fn(async () => structuredClone(authority)),
    commit: vi.fn(async (_kind, _id, request) => {
      if (request.expectedVersion !== authority.version) return { type: "conflict", requestId: request.id, snapshot: structuredClone(authority) };
      authority = { version: authority.version + 1, saved: request.saved as NoteDocumentV1, baseline: request.baseline as NoteDocumentV1 };
      return { type: "receipt", requestId: request.id, snapshot: structuredClone(authority) };
    }),
    readPreservation: vi.fn(async () => preservation),
    savePreservation: vi.fn(async (_kind, _id, value) => { preservation = structuredClone(value); return { revision: 1 }; }),
  };
  const controller = await AppNoteHbsSession.open({ loaded, gateway: hbs, onSnapshot: value => { current = value; },
    onStatus: vi.fn(), onDurable: vi.fn() });
  controller.start(); stops.push(() => controller.dispose());
  const begin = vi.spyOn(controller, "beginExternalChange");
  const install = vi.fn((_before: NoteDocumentV1, after: NoteDocumentV1) => () => { visible = after; return undefined; });
  const stopPort = registerTextInstallPort(pageId, { read: () => visible, isInstalling: false, prepare: install,
    prepareStructure: install, dispose() {} }); stops.push(stopPort);
  const saved = { ...loaded, page: { ...loaded.page, revision: 2 }, document: next };
  const loadDocument = vi.fn(async () => saved), saveDocument = vi.fn(), onError = vi.fn(), onChanged = vi.fn();
  const stopSession = registerNoteTextSession({ controller, gateway: { loadDocument, saveDocument } as unknown as NotesGateway,
    getSnapshot: () => current, onError, onChanged }); stops.push(stopSession);
  await vi.waitFor(() => expect(io.invoke).toHaveBeenCalledWith("agent_register_note_text_editor", expect.anything()));
  const sessionId = io.invoke.mock.calls.find(([name]) => name === "agent_register_note_text_editor")?.[1].sessionId;
  const pages = [{ pageId, revision: 2, document: next, review: { baseline: old, origin: "agent" as const, addToHistory: true } }];
  return { old, next, controller, begin, hbs, loadDocument, saveDocument, install, onError, onChanged, pages, stopPort, stopSession,
    state: () => controller.session.getSnapshot(), authority: () => authority, visible: () => visible, preserved: () => preservation,
    saved: () => { authority = { version: 2, saved: next, baseline: old }; },
    type: (text: string) => { visible = document(text); controller.enqueue(visible); },
    receipt: () => io.handlers.get("research-note-text-receipt")?.({ payload: { pages } }),
    emptyRelease: () => io.handlers.get("research-note-text-release")?.({ payload: { requestId: "original", pages: [] } }),
    flush: async () => {
      io.handlers.get("research-note-text-flush")?.({ payload: { requestId: "original", sessionId, pageId } });
      await vi.waitFor(() => expect(io.invoke).toHaveBeenCalledWith("agent_ack_note_text_editor", { requestId: "original", sessionId, error: null }));
    },
  };
}

it("delivers the original native receipt through the production HBS owner without another save or Keep", async () => {
  const s = await session(); await s.flush(); s.saved();
  s.receipt(); await deliverTextReceipt(s.pages); s.emptyRelease(); await deliverTextReceipt(s.pages);
  expect(s.state().content).toEqual(s.next); expect(s.state().baselineProjection).toEqual(s.old);
  expect(s.authority()).toEqual({ version: 2, saved: s.next, baseline: s.old });
  expect(s.begin).toHaveBeenCalledOnce(); expect(s.install).toHaveBeenCalledOnce();
  expect(s.hbs.commit).not.toHaveBeenCalled(); expect(s.saveDocument).not.toHaveBeenCalled();
  expect(s.onError).not.toHaveBeenCalled(); expect(s.controller.hasExternalChange()).toBe(false);
});

it("does not install a disposed HBS owner's receipt into a replacement view while S′ acknowledgement is delayed", async () => {
  const s = await session(); await s.flush(); s.saved();
  let acknowledge!: () => void;
  const pending = new Promise<void>(resolve => { acknowledge = resolve; });
  vi.mocked(s.hbs.savePreservation).mockImplementation(async () => { await pending; return { revision: 1 }; });
  const synchronize = vi.spyOn(s.controller, "synchronizeExternal");
  s.receipt();
  await vi.waitFor(() => {
    expect(s.state().observed.version).toBe(2);
    expect(s.hbs.savePreservation).toHaveBeenCalledWith("note",s.old.pageId,expect.objectContaining({ observed: expect.objectContaining({ version: 2 }) }));
  });
  const work = synchronize.mock.results[0]?.value;
  expect(work).toBeInstanceOf(Promise);
  s.stopSession(); s.stopPort(); s.controller.dispose();
  const replacement = document("Later input in replacement view"), install = vi.fn(() => () => undefined);
  stops.push(registerTextInstallPort(s.old.pageId,{ read: () => replacement, isInstalling:false,
    prepare:install, prepareStructure:install, dispose() {} }));
  acknowledge(); await work;
  expect(install).not.toHaveBeenCalled();
  expect(s.onChanged).not.toHaveBeenCalled(); expect(s.onError).not.toHaveBeenCalled();
});

it("preserves later H and S′ when an empty original release precedes the late native receipt", async () => {
  const s = await session(); await s.flush(); s.emptyRelease();
  await vi.waitFor(() => expect(isTextEditorHeld(s.old.pageId)).toBe(false));
  s.type("Later ordinary input"); const later = s.visible(); s.saved();
  s.receipt(); await deliverTextReceipt(s.pages); await s.controller.ensurePreserved();
  expect(s.state().content).toEqual(later); expect(s.visible()).toEqual(later);
  expect(s.preserved()).toMatchObject({ content: later });
  expect(s.install).not.toHaveBeenCalled(); expect(s.loadDocument).not.toHaveBeenCalled();
  expect(s.authority()).toEqual({ version: 2, saved: s.next, baseline: s.old });
  expect(s.hbs.commit).not.toHaveBeenCalled(); expect(s.controller.hasUnsavedChanges()).toBe(true);
  expect(s.onError).toHaveBeenCalledWith(expect.stringContaining("当前输入已保留"));
});
