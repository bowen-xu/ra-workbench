// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { NoteEditor } from "./NoteEditor";
import { noteBlockId, notePageId } from "@research-assistant/core/notes";
import type { NoteDocumentSnapshot, NotesGateway } from "@research-assistant/data/notes";
import { NoteSaveController } from "./noteSaveController";
import { registerNoteTextSession } from "./noteTextEditorSession";
import { registerNoteReviewSavePort, isTextEditorHeld, deliverTextReceipt, registerTextInstallPort, holdTextEditor, textInstallPort } from "./noteTextEditorPorts";
import { MockNoteTextSession } from "../../../../app/runtime/mock/MockNoteTextSession";

const f = vi.hoisted(() => ({ handlers: new Map<string, (event: { payload: unknown }) => void>(), invoke: vi.fn() }));
vi.mock("../../../runtime/platformEvents", () => ({ listen: async (name: string, handler: (event: { payload: unknown }) => void) => {
  f.handlers.set(name, handler); return () => f.handlers.delete(name);
} }));
vi.mock("../../../runtime/platformCore", () => ({ isDesktopRuntime: () => true, invoke: f.invoke }));
vi.mock("../NotesClient", async () => {
  const { NoteCommandHistory } = await import("./NoteCommandHistory");
  const history = new NoteCommandHistory();
  return { useNotesClient: () => ({ gateway: {}, history }), useNotesVersion: () => 0 };
});
const stops: (() => void | Promise<void>)[] = [];
afterEach(async () => { for (const stop of stops.splice(0).reverse()) await stop(); f.handlers.clear(); vi.clearAllMocks(); vi.unstubAllGlobals(); });
function snapshot(revision: number, text: string): NoteDocumentSnapshot {
  const pageId = notePageId("page");
  return { page: { id: pageId, schemaVersion: 1, title: "Page", revision, position: 0, status: "active", isFavorite: false,
    projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" }, document: { schemaVersion: 1, pageId, blocks: [{
      id: noteBlockId("block"), type: "paragraph", props: {}, children: [], content: { kind: "inline", items: [{ type: "text", text, styles: [] }] },
    }] } };
}
it("reconciles event and IPC receipts once through the same save lease", async () => {
  f.invoke.mockResolvedValue(undefined);
  let current = snapshot(1, "Old"); const saved = snapshot(2, "New"), install = vi.fn(() => undefined);
  const gateway = { loadDocument: vi.fn(async () => saved), saveDocument: vi.fn() } as unknown as NotesGateway;
  const onError = vi.fn();
  const controller = new NoteSaveController({ pageId: current.page.id, gateway, initialRevision: 1, onStatus: vi.fn(), onSnapshot: next => { current = next; } });
  stops.push(registerTextInstallPort("page", { isInstalling: false, read: () => current.document, prepare: () => install, prepareStructure: () => install, dispose: () => {} }));
  stops.push(registerNoteTextSession({ controller, gateway, getSnapshot: () => current, onError, onChanged: vi.fn() }));
  await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_register_note_text_editor", expect.anything()));
  const sessionId = f.invoke.mock.calls.find(([command]) => command === "agent_register_note_text_editor")?.[1].sessionId;
  f.handlers.get("research-note-text-flush")!({ payload: { requestId: "request", sessionId, pageId: "page" } });
  await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_ack_note_text_editor", { requestId: "request", sessionId, error: null }));
  const pages = [{ pageId: "page", revision: 2, document: saved.document }];
  f.handlers.get("research-note-text-release")!({ payload: { requestId: "request", pages } });
  await deliverTextReceipt(pages);
  await vi.waitFor(() => expect(install).toHaveBeenCalledOnce());
  await deliverTextReceipt(pages);
  expect(current).toEqual(saved); expect(onError).not.toHaveBeenCalled(); expect(controller.hasUnsavedChanges()).toBe(false);
});

it("keeps the real React editor's save fence valid through the native release", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); f.invoke.mockResolvedValue(undefined);
  let current = snapshot(1, "Old"); const saved = snapshot(2, "New"), onError = vi.fn();
  const gateway = { loadDocument: vi.fn(async () => saved), saveDocument: vi.fn() } as unknown as NotesGateway;
  let onStatus = () => {};
  const controller = new NoteSaveController({ pageId: current.page.id, gateway, initialRevision: 1, onStatus: () => onStatus(), onSnapshot: next => { current = next; } });
  const enqueue = vi.spyOn(controller, "enqueue");
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  function Harness() { const [, render] = useState(0); onStatus = () => render(value => value + 1);
    return <NoteEditor document={current.document} onChange={document => controller.enqueue(document)} />;
  }
  await act(async () => root.render(<Harness />));
  stops.push(async () => { await act(async () => root.unmount()); host.remove(); });
  stops.push(registerNoteTextSession({ controller, gateway, getSnapshot: () => current, onError, onChanged: vi.fn() }));
  await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_register_note_text_editor", expect.anything()));
  const sessionId = f.invoke.mock.calls.find(([command]) => command === "agent_register_note_text_editor")?.[1].sessionId;
  await act(async () => { f.handlers.get("research-note-text-flush")!({ payload: { requestId: "request", sessionId, pageId: "page" } }); });
  await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_ack_note_text_editor", { requestId: "request", sessionId, error: null }));
  expect(enqueue).not.toHaveBeenCalled();
  await act(async () => { await deliverTextReceipt([{ pageId: "page", revision: 2, document: saved.document }]); });
  expect(onError).not.toHaveBeenCalled(); expect(current).toEqual(saved); expect(host.textContent).toContain("New");
  expect(controller.hasUnsavedChanges()).toBe(false);
});

it("does not autosave an installed structural change after unlocking and permits undo", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const original = snapshot(1, "Old");
  const candidate = snapshot(2, "New");
  const added = { ...candidate.document.blocks[0]!, id: noteBlockId("added") };
  const saved = { ...candidate, document: { ...candidate.document, blocks: [...candidate.document.blocks, added] } };
  let current: NoteDocumentSnapshot = original, render = () => {};
  const controller = new NoteSaveController({ pageId: current.page.id, gateway: {} as NotesGateway, initialRevision: 1,
    onStatus: () => render(), onSnapshot: next => { current = next; } });
  const enqueue = vi.spyOn(controller, "enqueue");
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  function Harness() { const [, refresh] = useState(0); render = () => refresh(value => value + 1);
    return <NoteEditor document={current.document} onChange={document => controller.enqueue(document)} />;
  }
  stops.push(async () => { holdTextEditor("page", "install-test", false); await act(async () => root.unmount()); host.remove(); controller.dispose(); });
  await act(async () => root.render(<Harness />));
  const editor = host.querySelector('[contenteditable="true"]');
  for (const next of [saved, { ...original, page: { ...original.page, revision: 3 } }]) {
    const baseline = current;
    const lease = controller.beginExternalChange(baseline.page.revision, baseline.document);
    expect(lease).not.toBeNull();
    await act(async () => holdTextEditor("page", "install-test", true));
    expect(host.querySelector('[contenteditable="false"]')).not.toBeNull();
    const install = textInstallPort("page").prepareStructure(baseline.document, next.document);
    await act(async () => {
      expect(lease?.markDispatched()).toBe(true);
      expect(lease?.commitGroup(next, next.page.revision, install)).toBe(true);
    });
    await act(async () => holdTextEditor("page", "install-test", false));
    expect(enqueue).not.toHaveBeenCalled();
    expect(controller.hasUnsavedChanges()).toBe(false);
    expect(current).toEqual(next);
    expect(editor?.isConnected).toBe(true);
  }
  expect(host.textContent).toContain("Old");
  expect(host.textContent).not.toContain("New");
});

it("uses admitted candidate D to install final Undo all while formal S remains the revision baseline", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); f.invoke.mockResolvedValue(undefined);
  const old = snapshot(1, "Original argument");
  const source = { ...snapshot(1, "Original source").document.blocks[0]!, id: noteBlockId("source") };
  const original = { ...old, document: { ...old.document, blocks: [...old.document.blocks, source] } };
  const manualSource = { ...source, content: { kind: "inline" as const, items: [{ type: "text" as const, text: "Source + ordinary manual input", styles: [] }] } };
  const candidate = { ...original.document, blocks: [snapshot(1, "Agent argument").document.blocks[0]!, manualSource] };
  const keptBaseline = { ...original.document, blocks: [original.document.blocks[0]!, manualSource] };
  const result = { ...original, page: { ...original.page, revision: 2 }, document: keptBaseline };
  let current: NoteDocumentSnapshot = original, render = () => {};
  const gateway = { loadDocument: vi.fn(async () => result), saveDocument: vi.fn() } as unknown as NotesGateway;
  const onError = vi.fn();
  const controller = new NoteSaveController({ pageId: old.page.id, gateway, initialRevision: 1,
    onStatus: () => render(), onSnapshot: next => { current = next; } });
  const enqueue = vi.spyOn(controller, "enqueue");
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  function Harness() { const [, refresh] = useState(0); render = () => refresh(value => value + 1);
    return <NoteEditor document={current.document} onChange={document => controller.enqueue(document)} />;
  }
  await act(async () => root.render(<Harness />));
  stops.push(async () => { await act(async () => root.unmount()); host.remove(); controller.dispose(); });
  const editor = host.querySelector(".ProseMirror");
  await act(async () => textInstallPort("page").prepareStructure(original.document, candidate,
    { baseline: keptBaseline, origin: "agent", addToHistory: false })());
  expect(current).toEqual(original);
  expect(textInstallPort("page").read().blocks[0]?.content).toEqual(candidate.blocks[0]?.content);
  stops.push(registerNoteTextSession({ controller, gateway, getSnapshot: () => current, onError, onChanged: vi.fn() }));
  await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_register_note_text_editor", expect.anything()));
  const sessionId = f.invoke.mock.calls.find(([command]) => command === "agent_register_note_text_editor")?.[1].sessionId;
  await act(async () => { f.handlers.get("research-note-text-flush")!({ payload: { requestId: "undo-all", sessionId, pageId: "page" } }); });
  await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_ack_note_text_editor", { requestId: "undo-all", sessionId, error: null }));
  await act(async () => { await deliverTextReceipt([{ pageId: "page", revision: 2, document: result.document }]); });
  expect(onError).not.toHaveBeenCalled();
  expect(current).toEqual(result);
  expect(host.textContent).toContain("Original argument");
  expect(host.textContent).toContain("ordinary manual input");
  expect(host.textContent).not.toContain("Agent argument");
  expect(editor?.isConnected).toBe(true);
  expect(enqueue).not.toHaveBeenCalled();
  expect(controller.hasUnsavedChanges()).toBe(false);
});

it("retains the original admitted D during recovery and never adopts later input as a new baseline", async () => {
  f.invoke.mockResolvedValue(undefined);
  let current = snapshot(1, "Saved S");
  const admitted = snapshot(1, "Candidate D").document, later = snapshot(1, "Later input").document, result = snapshot(2, "Final result");
  let live = admitted;
  const prepare = vi.fn((before, after) => {
    if (live !== before) throw new Error("Live editor changed after admission");
    return () => { if (live !== before) throw new Error("Late input"); live = after; return undefined; };
  });
  const gateway = { loadDocument: vi.fn(async () => result), saveDocument: vi.fn() } as unknown as NotesGateway;
  const onError = vi.fn();
  const controller = new NoteSaveController({ pageId: current.page.id, gateway, initialRevision: 1, onStatus: vi.fn(), onSnapshot: next => { current = next; } });
  stops.push(() => controller.dispose());
  stops.push(registerTextInstallPort("page", { isInstalling: false, read: () => live, prepare, prepareStructure: prepare, dispose: () => {} }));
  stops.push(registerNoteTextSession({ controller, gateway, getSnapshot: () => current, onError, onChanged: vi.fn() }));
  await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_register_note_text_editor", expect.anything()));
  const sessionId = f.invoke.mock.calls.find(([command]) => command === "agent_register_note_text_editor")?.[1].sessionId;
  f.handlers.get("research-note-text-flush")!({ payload: { requestId: "captured", sessionId, pageId: "page" } });
  await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_ack_note_text_editor", { requestId: "captured", sessionId, error: null }));
  live = later;
  const pages = [{ pageId: "page", revision: 2, document: result.document }];
  await deliverTextReceipt(pages);
  await deliverTextReceipt(pages);
  expect(prepare).toHaveBeenCalledTimes(2);
  expect(prepare.mock.calls.every(([before]) => before === admitted)).toBe(true);
  expect(live).toBe(later); expect(current.page.revision).toBe(1);
  expect(onError).toHaveBeenCalled(); expect(controller.hasUnsavedChanges()).toBe(true);
});

it("refuses an unleased receipt over an unrelated later candidate", async () => {
  f.invoke.mockResolvedValue(undefined);
  const current = snapshot(1, "Saved S"), result = snapshot(2, "Native result"), later = snapshot(1, "Unrelated later input").document;
  const install = vi.fn(() => undefined), prepare = vi.fn(() => install), onError = vi.fn();
  const controller = new NoteSaveController({ pageId: current.page.id, gateway: {} as NotesGateway, initialRevision: 1, onStatus: vi.fn(), onSnapshot: vi.fn() });
  stops.push(() => controller.dispose());
  stops.push(registerTextInstallPort("page", { isInstalling: false, read: () => later, prepare, prepareStructure: prepare, dispose: () => {} }));
  stops.push(registerNoteTextSession({ controller, gateway: {} as NotesGateway, getSnapshot: () => current, onError, onChanged: vi.fn() }));
  await deliverTextReceipt([{ pageId: "page", revision: 2, document: result.document }]);
  expect(prepare).not.toHaveBeenCalled(); expect(install).not.toHaveBeenCalled();
  expect(onError).toHaveBeenCalledWith(expect.stringContaining("当前输入已保留"));
});

it("the mock save session installs over admitted D while preserving the original formal revision guard", async () => {
  let current = snapshot(1, "Formal S");
  const candidate = snapshot(1, "Candidate D").document, result = snapshot(2, "Undo D with manual input");
  let live = candidate;
  const install = vi.fn(() => { live = result.document; return undefined; });
  const prepare = vi.fn(() => install);
  const gateway = { saveDocument: vi.fn(async () => result) } as unknown as NotesGateway;
  const controller = new NoteSaveController({ pageId: current.page.id, gateway, initialRevision: 1, onStatus: vi.fn(), onSnapshot: next => { current = next; } });
  stops.push(() => controller.dispose());
  stops.push(registerTextInstallPort("page", { isInstalling: false, read: () => live, prepare, prepareStructure: prepare, dispose: () => {} }));
  const mock = new MockNoteTextSession(gateway, "page");
  stops.push(mock.register({ controller, gateway, getSnapshot: () => current, onError: vi.fn(), onChanged: vi.fn() }));
  await mock.apply(result.document, 1);
  expect(prepare).toHaveBeenCalledWith(candidate, result.document, undefined);
  expect(gateway.saveDocument).toHaveBeenCalledWith({ document: result.document, expectedRevision: 1, saveReason: "structure" });
  expect(current).toEqual(result); expect(live).toEqual(result.document);
  expect(install).toHaveBeenCalledOnce(); expect(controller.hasUnsavedChanges()).toBe(false);
});

it("the preview save preflight can fail and retry without leaving a dispatched lease", async () => {
  let current = snapshot(1, "Old");
  const saved = snapshot(2, "New"), install = vi.fn(() => undefined);
  const gateway = { saveDocument: vi.fn(async () => saved) } as unknown as NotesGateway;
  const controller = new NoteSaveController({ pageId: current.page.id, gateway, initialRevision: 1,
    onStatus: vi.fn(), onSnapshot: next => { current = next; } });
  stops.push(() => controller.dispose());
  stops.push(registerTextInstallPort("page", { isInstalling: false, read: () => current.document,
    prepare: () => install, prepareStructure: () => install, dispose() {} }));
  const beforeSave = vi.fn().mockRejectedValueOnce(new Error("模拟保存失败")).mockResolvedValue(undefined);
  const mock = new MockNoteTextSession(gateway, "page", beforeSave);
  stops.push(mock.register({ controller, gateway, getSnapshot: () => current, onError: vi.fn(), onChanged: vi.fn() }));
  await expect(mock.apply(saved.document, 1)).rejects.toThrow("模拟保存失败");
  expect(gateway.saveDocument).not.toHaveBeenCalled(); expect(install).not.toHaveBeenCalled();
  expect(controller.hasUnsavedChanges()).toBe(false);
  await expect(mock.apply(saved.document, 1)).resolves.toEqual(saved);
  expect(beforeSave).toHaveBeenCalledTimes(2); expect(gateway.saveDocument).toHaveBeenCalledOnce();
  expect(install).toHaveBeenCalledOnce(); expect(current).toEqual(saved);
});

it("installs a saved Agent body and its pending baseline in one editor operation", async () => {
  f.invoke.mockResolvedValue(undefined);
  let current = snapshot(1, "Old"), live = current.document;
  const saved = snapshot(2, "Agent D");
  const install = vi.fn(() => { live = saved.document; return undefined; }), prepare = vi.fn(() => install);
  const gateway = { loadDocument: vi.fn(async () => saved) } as unknown as NotesGateway;
  const onError = vi.fn(), controller = new NoteSaveController({ pageId: current.page.id, gateway, initialRevision: 1,
    onStatus: vi.fn(), onSnapshot: next => { current = next; } });
  stops.push(() => controller.dispose());
  stops.push(registerTextInstallPort("page", { isInstalling: false, read: () => live, prepare, prepareStructure: prepare, dispose() {} }));
  stops.push(registerNoteTextSession({ controller, gateway, getSnapshot: () => current, onError, onChanged: vi.fn() }));
  const review = { baseline: current.document, origin: "agent" as const, addToHistory: true };
  const pages = [{ pageId: "page", revision: 2, document: saved.document, review }];
  await deliverTextReceipt(pages); await deliverTextReceipt(pages);
  expect(prepare).toHaveBeenCalledExactlyOnceWith(expect.anything(), saved.document, review);
  expect(install).toHaveBeenCalledOnce(); expect(current.page.revision).toBe(2);
  expect(onError).not.toHaveBeenCalled();
});

it("acknowledges a typing checkpoint without reinstalling it over later keystrokes", async () => {
  f.invoke.mockResolvedValue(undefined);
  let current = snapshot(1, "Old");
  const saved = snapshot(2, "First input"), later = snapshot(1, "First input and newer keystrokes").document;
  const prepare = vi.fn(() => () => undefined), onError = vi.fn();
  const gateway = { loadDocument: vi.fn(async () => saved) } as unknown as NotesGateway;
  const controller = new NoteSaveController({ pageId: current.page.id, gateway, initialRevision: 1,
    onStatus: vi.fn(), onSnapshot: next => { current = next; } });
  stops.push(() => controller.dispose());
  stops.push(registerTextInstallPort("page", { isInstalling: false, read: () => later, prepare, prepareStructure: prepare, dispose() {} }));
  stops.push(registerNoteTextSession({ controller, gateway, getSnapshot: () => current, onError, onChanged: vi.fn() }));
  await deliverTextReceipt([{ pageId: "page", revision: 2, document: saved.document,
    review: { baseline: current.document, origin: "manual", addToHistory: false } }]);
  expect(current).toBe(saved); expect(prepare).not.toHaveBeenCalled(); expect(onError).not.toHaveBeenCalled();
});

it.each(["native", "preview"] as const)("keeps the editor editable throughout a delayed %s typing acknowledgment", async mode => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); f.invoke.mockResolvedValue(undefined);
  let current = snapshot(1, "Old"), finish!: (value: NoteDocumentSnapshot) => void;
  const saved = snapshot(2, "First input");
  const response = new Promise<NoteDocumentSnapshot>(resolve => { finish = resolve; });
  const gateway = { loadDocument: vi.fn(() => response), saveDocument: vi.fn(() => response) } as unknown as NotesGateway;
  const onError = vi.fn();
  let render = () => {};
  const controller = new NoteSaveController({ pageId: current.page.id, gateway, initialRevision: 1,
    onStatus: () => render(), onSnapshot: next => { current = next; } });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  function Harness() { const [, refresh] = useState(0); render = () => refresh(value => value + 1);
    return <NoteEditor document={current.document} onChange={vi.fn()} />;
  }
  await act(async () => root.render(<Harness />));
  stops.push(async () => { await act(async () => root.unmount()); host.remove(); controller.dispose(); });
  await act(async () => textInstallPort("page").prepareStructure(current.document, saved.document,
    { baseline: current.document, origin: "agent", addToHistory: false })());
  const editor = host.querySelector<HTMLElement>(".bn-editor")!;
  editor.focus();
  const options = { controller, gateway, getSnapshot: () => current, onError, onChanged: vi.fn() };
  const review = { baseline: current.document, origin: "manual" as const, addToHistory: false };
  let operation!: Promise<unknown>;
  if (mode === "native") {
    stops.push(registerNoteTextSession(options));
    await act(async () => { operation = deliverTextReceipt([{ pageId: "page", revision: 2, document: saved.document, review }]); });
  } else {
    const mock = new MockNoteTextSession(gateway, "page"); stops.push(mock.register(options));
    await act(async () => { operation = mock.apply(saved.document, 1, review); });
  }
  const during = { editable: editor.contentEditable, focused: document.activeElement === editor };
  await act(async () => { finish(saved); await operation; });
  expect(during).toEqual({ editable: "true", focused: true });
  expect(host.querySelector(".bn-editor")).toBe(editor);
  expect(current).toEqual(saved); expect(onError).not.toHaveBeenCalled();
});

it("fences input before native admission and holds it until the review save owner takes over", async () => {
  f.invoke.mockResolvedValue(undefined);
  let current = snapshot(1, "Original"), finishFlush!: () => void, finishHandoff!: () => void;
  const flush = new Promise<void>(resolve => { finishFlush = resolve; });
  const handoff = new Promise<void>(resolve => { finishHandoff = resolve; });
  const saved = snapshot(2, "Agent result"), setWriteLocked = vi.fn();
  const gateway = { loadDocument: vi.fn(async () => saved) } as unknown as NotesGateway;
  const controller = new NoteSaveController({ pageId: current.page.id, gateway, initialRevision: 1,
    onStatus: vi.fn(), onSnapshot: value => { current = value; } });
  stops.push(() => controller.dispose());
  stops.push(registerTextInstallPort("page", { setWriteLocked, isInstalling: false, read: () => current.document,
    prepare: () => () => undefined, prepareStructure: () => () => undefined, dispose() {} }));
  const synchronize = vi.fn(() => handoff);
  stops.push(registerNoteReviewSavePort("page", { flush: () => flush, retry: vi.fn(), getState: () => ({ status: "saved" }), synchronize }));
  stops.push(registerNoteTextSession({ controller, gateway, getSnapshot: () => current, onError: vi.fn(), onChanged: vi.fn() }));
  await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_register_note_text_editor", expect.anything()));
  const sessionId = f.invoke.mock.calls.find(([command]) => command === "agent_register_note_text_editor")?.[1].sessionId;
  f.handlers.get("research-note-text-flush")!({ payload: { requestId: "agent", sessionId, pageId: "page" } });
  expect(setWriteLocked).toHaveBeenLastCalledWith(true);
  expect(isTextEditorHeld("page")).toBe(true);
  expect(f.invoke.mock.calls.some(([name]) => name === "agent_ack_note_text_editor")).toBe(false);
  finishFlush();
  await vi.waitFor(() => expect(f.invoke).toHaveBeenCalledWith("agent_ack_note_text_editor", { requestId: "agent", sessionId, error: null }));
  const applying = deliverTextReceipt([{ pageId: "page", revision: 2, document: saved.document,
    review: { baseline: snapshot(1, "Original").document, origin: "agent", addToHistory: true } }]);
  await vi.waitFor(() => expect(synchronize).toHaveBeenCalledOnce());
  expect(isTextEditorHeld("page")).toBe(true);
  finishHandoff(); await applying;
  expect(isTextEditorHeld("page")).toBe(false);
  expect(setWriteLocked).toHaveBeenLastCalledWith(false);
});
