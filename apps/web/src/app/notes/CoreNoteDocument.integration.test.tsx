// @vitest-environment happy-dom
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {createNoteDocumentFromText, noteBlockId, noteDocumentPlainText} from "@research-assistant/core/notes";
import {NotesProvider} from "../../core/features/notes/NotesClient";
import {browserNotesGateway} from "../../core/features/notes/gateways/browserNotesGateway";
import {configureBrowserNotesDatabase} from "../../core/features/notes/gateways/browserNotesDatabase";
import {createMemoryNotesDatabase} from "../../core/runtime/testing/memoryNotesDatabase";
import {createTestNamespacedStorage} from "../../core/runtime/testing/createTestNamespacedStorage";
import {selectAppRuntime} from "../../core/runtime/appRuntime";
import {getHbsDocumentGateway} from "../../core/services/hbsDocumentGateway";
import {CoreNoteWorkspaceHost, RetainedNoteEditors, useNoteDocumentSession} from "../../core/public/noteDocuments";
import {useNoteResources, type NoteResourceAccess} from "../../core/public/noteResources";
import type {NoteEditorInstance} from "../../core/features/notes/editor/noteEditorSchema";

// The test has no native Agent backend. Only its proposal discovery crosses IPC;
// document reads, HBS commits, preservation and revisions use the real Core ports.
vi.mock("@tauri-apps/api/core", async original => ({
  ...await original<typeof import("@tauri-apps/api/core")>(),
  invoke: vi.fn(async (command: string) => {
    if (command === "agent_note_text_patch_conversations") return [];
    throw new Error(`Unexpected native command in Core note integration: ${command}`);
  }),
}));
// A single Core document session must not evaluate the management plugin.
vi.mock("../../plugins/note_manager/public/plugin", () => {throw new Error("note_manager declaration evaluated");});
vi.mock("@research-assistant/note-manager-plugin/workspace", () => {throw new Error("note_manager workspace evaluated");});
vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});

let root: Root | undefined, host: HTMLDivElement;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.clear();
  const storage = createTestNamespacedStorage(localStorage, "core-note-integration:");
  selectAppRuntime({mode: "browser", storage, gateways: {}, dispose() {}});
  configureBrowserNotesDatabase(createMemoryNotesDatabase().open);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {
  if (root) await act(async () => root!.unmount());
  root = undefined; host.remove(); vi.unstubAllGlobals();
});
async function frame() {
  await act(async () => {await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));});
}

it("edits, saves, replays history and reopens the same Core note without the management plugin", async () => {
  const onError = vi.fn(), onNotice = vi.fn(), onLoaded = vi.fn();
  let current!: ReturnType<typeof useNoteDocumentSession>, resources!: NoteResourceAccess, editor!: NoteEditorInstance;
  const ready = (value: NoteEditorInstance) => {editor = value;};
  function SingleDocument() {
    resources = useNoteResources();
    current = useNoteDocumentSession({onError, onNotice, onLoaded});
    return <RetainedNoteEditors sessions={current.retainedSessions} activeId={current.snapshot?.page.id}
      conflictLocked={false} onChange={current.onRetainedDocumentChange} onEditorReady={ready}/>;
  }
  const render = async (key: number) => {
    await act(async () => root!.render(<NotesProvider gateway={browserNotesGateway}><CoreNoteWorkspaceHost key={key}><SingleDocument/></CoreNoteWorkspaceHost></NotesProvider>));
    await frame();
  };
  await render(0);
  const originalResources = resources, originalHistory = resources.history;
  const created = await resources.mutate(gateway => gateway.createPage({title: "Core-owned note"}));
  const id = created.page.id;
  const initial = {...createNoteDocumentFromText(id, "Original body", () => noteBlockId("body")), title: created.page.title};
  const hbs = getHbsDocumentGateway();
  await hbs.commit("note", id, {id: "core-integration-seed", expectedVersion: created.page.revision, saved: initial, baseline: initial});
  await act(async () => current.loadPage(id)); await frame();
  const owner = current.saveControllerRef.current, view = editor.prosemirrorView, sessionId = current.editorSessionId;
  expect(Object.isFrozen(current.saveControllerRef)).toBe(true);
  expect(Reflect.set(current.saveControllerRef, "current", undefined)).toBe(false);
  expect(current.saveControllerRef.current).toBe(owner);
  expect(current).not.toHaveProperty("installSaveController");
  expect(host.querySelector(".ProseMirror")).toBe(view.dom);
  expect(current.snapshot?.document.pageId).toBe(id);
  const initialSnapshot = await resources.gateway.loadDocument(id);
  const handle = current.retainedSessions[0];
  expect(handle).toBeDefined();
  expect(Object.isFrozen(handle)).toBe(true);
  expect(Object.keys(handle!)).toEqual(["snapshot", "editorSessionId", "saveStatus", "saveMessage"]);
  expect(handle).not.toHaveProperty("controller");
  expect(handle).not.toHaveProperty("hbs");
  // Equal metadata cannot forge a Core editor owner.
  expect(() => RetainedNoteEditors({sessions: [{snapshot: initialSnapshot, editorSessionId: sessionId,
    saveStatus: "saved", saveMessage: undefined}], activeId: id, conflictLocked: false, onChange() {}}))
    .toThrow("NOTE_SESSION_OWNER_UNAVAILABLE");
  expect(initialSnapshot.document).toEqual(initial);
  resources.history.closeGroups();
  await act(async () => editor.updateBlock("body", {content: "Edited in Core"})); await frame();
  await act(async () => {const durable = await current.ensureDurable(true); expect(durable, JSON.stringify({errors:onError.mock.calls,status:current.saveStatus,message:current.saveMessage})).toBe(true);}); await frame();
  const saved = await resources.gateway.loadDocument(id);
  expect(noteDocumentPlainText(saved.document)).toContain("Edited in Core");
  expect(saved.page.revision).toBeGreaterThan(created.page.revision);
  expect(current.saveControllerRef.current).toBe(owner);
  expect(current.editorSessionId).toBe(sessionId);
  expect(current.retainedSessions[0]).toBe(handle);
  expect(handle?.snapshot).toBe(current.snapshot);
  expect(editor.prosemirrorView).toBe(view);
  expect(resources).toBe(originalResources); expect(resources.history).toBe(originalHistory);
  resources.history.closeGroups();
  await act(async () => {expect(await resources.history.run("undo", undefined, view.dom)).toBe(true);}); await frame();
  expect(noteDocumentPlainText(current.snapshot!.document)).toContain("Original body");
  await act(async () => {expect(await current.ensureDurable(true)).toBe(true);});
  await act(async () => {expect(await resources.history.run("redo", undefined, view.dom)).toBe(true);}); await frame();
  await act(async () => {expect(await current.ensureDurable(true)).toBe(true);}); await frame();
  expect(editor.prosemirrorView).toBe(view);
  const final = await browserNotesGateway.loadDocument(id), revisions = await resources.gateway.listRevisions(id);
  expect(noteDocumentPlainText(final.document)).toContain("Edited in Core");
  // Revision history retains replaced snapshots; the latest version is read
  // from the canonical document rather than invented as another history entry.
  const originalRevision = revisions.find(revision => revision.pageRevision === initialSnapshot.page.revision);
  expect(originalRevision).toBeDefined();
  expect(await resources.gateway.loadRevision(id, originalRevision!.id)).toEqual(initial);
  expect((await hbs.readPreservation("note", id))).toMatchObject({content: final.document});
  await render(1);
  await act(async () => current.loadPage(id)); await frame();
  expect(current.snapshot?.page.id).toBe(id);
  expect(current.snapshot?.page.revision).toBe(final.page.revision);
  expect(current.snapshot?.document).toEqual(final.document);
  expect(editor.prosemirrorView).not.toBe(view);
  expect(resources).toBe(originalResources); expect(resources.history).toBe(originalHistory);
  expect((await resources.gateway.listRevisions(id)).map(revision => revision.id)).toEqual(revisions.map(revision => revision.id));
  expect(onError).not.toHaveBeenCalled();
});
