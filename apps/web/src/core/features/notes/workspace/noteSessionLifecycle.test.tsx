// @vitest-environment happy-dom
import { act, Activity, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId } from "@research-assistant/core/notes";
import { type NoteDocumentSnapshot, type NotesGateway } from "@research-assistant/data/notes";
import { NotesClient } from "../NotesClient";
import { useNoteDocumentSession } from "./useNoteDocumentSession";
import { holdTextEditor, holdNoteEditorInput, isTextEditorHeld, isTextEditorSaveBlocked, registerTextInstallPort, notifyNoteReviewSaveState, registerNoteReviewSavePort, registerNoteReviewStateContributor, synchronizeNoteReview, type NoteReviewSaveState } from "../editor/noteTextEditorPorts";
import { getAppStorage, selectAppStorage } from "../../../runtime/appStorage";
import { createIndexedPreviewStorage } from "../../../runtime/previewIndexedStorage";
import { configurePreviewDatabase } from "../../../runtime/previewIndexedDatabase";
import { createMemoryPreviewDatabase } from "../../../../app/runtime/testing/memoryPreviewDatabase";
import { storagePersistence } from "../../../runtime/storagePersistence";
import type { AppHbsDocumentGateway } from "../../../services/hbsDocumentGateway";
import type { DocumentSnapshot } from "@research-assistant/core/hbs";

const f = vi.hoisted(() => ({ saveNote: vi.fn(), loadNote: vi.fn(), clearNote: vi.fn() }));
vi.mock("./noteRecovery", () => ({ saveNoteRecoveryDraft: f.saveNote, loadNoteRecoveryDraft: f.loadNote, clearNoteRecoveryDraft: f.clearNote }));
const cleanups: (() => Promise<void>)[] = [];
beforeEach(() => { vi.resetAllMocks(); vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); });
afterEach(async () => { for (const dispose of cleanups.splice(0)) await dispose(); vi.unstubAllGlobals(); });
async function mount<T>(hook: () => T) {
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host); let current!: T;
  function Harness() { current = hook(); return null; }
  await act(async () => root.render(<StrictMode><Harness /></StrictMode>));
  const unmount = async () => { await act(async () => root.unmount()); host.remove(); };
  cleanups.push(unmount);
  return { get: () => current, unmount };
}
function deferred<T>() {
  let resolve!: (value: T) => void, reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
function snapshot(id = "page-1", revision = 3, text = "Saved"): NoteDocumentSnapshot {
  return { page: { id: notePageId(id), schemaVersion: 1, title: id, revision, position: 0, status: "active", isFavorite: false,
    projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" }, document: { schemaVersion: 1, pageId: notePageId(id), blocks: [{
      id: noteBlockId("block"), type: "paragraph", props: {}, children: [], content: { kind: "inline", items: [{ type: "text", text, styles: [] }] },
    }] } };
}
function pageFixture() {
  const gateway = { loadDocument: vi.fn(async (id: string) => snapshot(id)), listOutgoing: vi.fn(async () => []), listBacklinks: vi.fn(async () => []),
    saveDocument: vi.fn(async (request: { document: NoteDocumentSnapshot["document"]; expectedRevision: number }) => ({ ...snapshot(request.document.pageId, request.expectedRevision + 1), document: request.document })) };
  const documents = new Map<string, DocumentSnapshot<NoteDocumentSnapshot["document"]>>(), preserved = new Map<string, unknown>();
  const hbsGateway: AppHbsDocumentGateway = {
    read: vi.fn(async (_kind, id) => {
      const index = gateway.loadDocument.mock.calls.reduce((last, call, position) => call[0] === id ? position : last, -1);
      const loaded = index >= 0 ? await gateway.loadDocument.mock.results[index]!.value as NoteDocumentSnapshot : snapshot(id);
      let authority = documents.get(id);
      if (!authority || loaded.page.revision > authority.version) {
        authority = { version: loaded.page.revision, saved: loaded.document, baseline: authority?.baseline ?? loaded.document }; documents.set(id, authority);
      }
      return authority;
    }),
    commit: vi.fn(async (_kind, id, request) => {
      const saved = await gateway.saveDocument({ document: request.saved as NoteDocumentSnapshot["document"], expectedRevision: request.expectedVersion });
      const authority = { version: saved.page.revision, saved: saved.document, baseline: request.baseline as NoteDocumentSnapshot["document"] }; documents.set(id, authority);
      return { type: "receipt", requestId: request.id, snapshot: authority };
    }),
    readPreservation: vi.fn(async (_kind, id) => preserved.get(id) ?? null),
    savePreservation: vi.fn(async (_kind, id, checkpoint) => { preserved.set(id, checkpoint); return { revision: 1 }; }),
  };
  const options = { client: new NotesClient(gateway as unknown as NotesGateway), hbsGateway, onError: vi.fn(), onNotice: vi.fn(), onLoaded: vi.fn() };
  return { gateway, options, hbsGateway, documents, preserved };
}
it("explicitly saves conflicted input to S′ without treating preservation as Keep",async()=>{
  const f=pageFixture(),id=notePageId("page-1"),base=snapshot(id,3,"80%").document;
  f.documents.set(id,{version:4,saved:snapshot(id,4,"85%").document,baseline:base});
  const h=await mount(()=>useNoteDocumentSession(f.options));
  await act(async()=>h.get().loadPage(id));
  const owner=h.get().retainedSessions[0]!,mine=snapshot(id,4,"185%").document;
  await act(async()=>{
    owner.hbs.edit(mine);
    owner.hbs.receive({version:5,saved:snapshot(id,5,"90%").document,baseline:base});
  });
  expect(h.get().saveStatus).toBe("conflict");
  await act(async()=>expect(await h.get().saveExplicitly()).toBe("preserved"));
  expect(f.preserved.get(id)).toMatchObject({content:mine});
  expect(owner.hbs.getSnapshot().conflicts.length).toBeGreaterThan(0);
  expect(owner.hbs.getSnapshot().baselineProjection).toEqual(base);
  expect(f.hbsGateway.commit).not.toHaveBeenCalled();
});
it("does not write the recovery draft per keystroke and cancels idle timers after a successful switch flush", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  vi.useFakeTimers();
  try {
    for (let index = 0; index < 5; index++) {
      await act(async () => h.get().onDocumentChange(snapshot("page-1", 3, `Edit ${index}`).document));
      await act(async () => { await vi.advanceTimersByTimeAsync(1000); });
    }
    expect(f.saveNote).not.toHaveBeenCalled(); expect(fixture.gateway.saveDocument).not.toHaveBeenCalled();
    await act(async () => { await vi.advanceTimersByTimeAsync(1999); });
    expect(f.saveNote).not.toHaveBeenCalled();
    await act(async () => { await vi.advanceTimersByTimeAsync(1); });
    expect(fixture.preserved.get("page-1")).toMatchObject({content: snapshot("page-1", 3, "Edit 4").document}); expect(fixture.gateway.saveDocument).toHaveBeenCalledOnce();
    await act(async () => h.get().onDocumentChange(snapshot("page-1", 4, "Switch now").document));
    await act(async () => { expect(await h.get().ensureDurable()).toBe(true); });
    await act(async () => { await vi.advanceTimersByTimeAsync(3000); });
    expect(f.saveNote).not.toHaveBeenCalled(); expect(fixture.gateway.saveDocument).toHaveBeenCalledTimes(2);
  } finally { vi.useRealTimers(); }
});
it("loads a page, keeps its editor session across saves, and releases the original save owner on replacement", async () => {
  const f = pageFixture(), h = await mount(() => useNoteDocumentSession(f.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  const original = h.get().saveControllerRef.current, session = h.get().editorSessionId;
  const edited = snapshot("page-1", 3, "Edited").document;
  await act(async () => { h.get().onDocumentChange(edited); await h.get().saveControllerRef.current?.flush(); });
  expect(h.get().snapshot?.document).toEqual(edited); expect(h.get().snapshot?.page.revision).toBe(4);
  expect(h.get().editorSessionId).toBe(session); expect(h.get().saveStatus).toBe("saved");
  await act(async () => h.get().loadPage(notePageId("page-2")));
  expect(h.get().saveControllerRef.current).not.toBe(original); expect(h.get().editorSessionId).toBe(session + 1);
  await original?.retry(); expect(f.gateway.saveDocument).toHaveBeenCalledOnce();
});

it("flushes real HBS input while metadata freezes interaction, and still blocks an external writer", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  const latest = snapshot("page-1", 3, "input before delete").document;
  await act(async () => h.get().onDocumentChange(latest));
  const setWriteLocked = vi.fn();
  const unregister = registerTextInstallPort("page-1", {read: () => latest, setWriteLocked,
    isInstalling: false, prepare: () => () => undefined, prepareStructure: () => () => undefined, dispose: () => {}});
  let release!: () => void;
  try {
    await act(async () => {release = holdNoteEditorInput("metadata-delete");});
    expect(isTextEditorHeld("page-1")).toBe(true); expect(isTextEditorSaveBlocked("page-1")).toBe(false);
    await act(async () => {expect(await h.get().ensureDurable()).toBe(true);});
    expect(fixture.gateway.saveDocument).toHaveBeenCalledOnce();
    expect(fixture.gateway.saveDocument.mock.calls[0]![0].document).toEqual(latest);
    expect(fixture.preserved.get("page-1")).toMatchObject({content: latest});
    await act(async () => holdTextEditor("page-1", "external-writer", true));
    expect(isTextEditorSaveBlocked("page-1")).toBe(true);
    await act(async () => {expect(await h.get().ensureDurable()).toBe(false);});
  } finally {
    await act(async () => {holdTextEditor("page-1", "external-writer", false); release?.();});
    unregister();
  }
  expect(isTextEditorHeld("page-1")).toBe(false); expect(setWriteLocked).toHaveBeenLastCalledWith(false);
});

it("keeps acknowledged page metadata after HBS synchronizes and republishes the document", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  const initial = h.get().snapshot!, editorSessionId = h.get().editorSessionId;
  for (const [index, status] of ["archived", "trashed", "active"].entries()) {
    const page = { ...initial.page, revision: initial.page.revision + index + 1, status: status as typeof initial.page.status,
      title: `Acknowledged ${status}`, isFavorite: true };
    fixture.documents.set(page.id, { version: page.revision, saved: initial.document, baseline: initial.document });
    await act(async () => {
      h.get().updateMetadata(page);
      await synchronizeNoteReview(page.id);
    });
    expect(h.get().snapshot?.page).toEqual(page);
    await act(async () => { await h.get().saveControllerRef.current?.flush(); });
    expect(h.get().snapshot?.page).toEqual(page);
    expect(h.get().editorSessionId).toBe(editorSessionId);
    expect(h.get().snapshot?.document).toEqual(initial.document);
  }
});

it("combines review progress with the existing save owner without an older receipt or another page showing saved", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  let state: NoteReviewSaveState = { status: "dirty" };
  const retry = vi.fn(async () => { state = { status: "saved" }; notifyNoteReviewSaveState(); });
  let unregister!: () => void;
  await act(async () => { unregister = registerNoteReviewSavePort("page-1", { getState: () => state, flush: vi.fn(), retry }); });
  cleanups.push(async () => unregister());
  expect(h.get().saveStatus).toBe("dirty");
  await act(async () => {
    const lease = h.get().saveControllerRef.current!.beginExternalChange(3, snapshot().document)!;
    lease.markDispatched();
    fixture.documents.set("page-1", {version:4,saved:snapshot("page-1",4,"Older checkpoint").document,baseline:snapshot().document});
    await h.get().saveControllerRef.current!.synchronizeExternal?.(snapshot("page-1",4,"Older checkpoint"));
    lease.commit(snapshot("page-1", 4, "Older checkpoint"), () => undefined);
  });
  expect(h.get().saveStatus).toBe("dirty");
  await act(async () => { state = { status: "saving" }; notifyNoteReviewSaveState(); });
  expect(h.get().saveStatus).toBe("saving");
  await act(async () => { state = { status: "failed", message: "Unconfirmed review receipt" }; notifyNoteReviewSaveState(); });
  expect(h.get().saveStatus).toBe("failed"); expect(h.get().saveMessage).toBe("Unconfirmed review receipt");
  await act(async () => { await h.get().retrySave(); });
  expect(retry).toHaveBeenCalledOnce(); expect(h.get().saveStatus).toBe("saved");
  await act(async () => { state = { status: "dirty" }; notifyNoteReviewSaveState(); });
  await act(async () => h.get().loadPage(notePageId("page-2")));
  expect(h.get().saveStatus).toBe("saved");
});

it("keeps both original request identities in the header when HBS and metadata are equally failed",async()=>{
  const fixture=pageFixture(),h=await mount(()=>useNoteDocumentSession(fixture.options));
  await act(async()=>h.get().loadPage(notePageId("page-1")));
  fixture.gateway.saveDocument.mockRejectedValueOnce(new Error("body transport unknown"));
  await act(async()=>{h.get().onDocumentChange(snapshot("page-1",3,"input").document);await h.get().saveControllerRef.current?.flush();});
  const request=vi.mocked(fixture.hbsGateway.commit).mock.calls[0]![2];
  let metadata:NoteReviewSaveState={status:"failed",message:"MODEL_METADATA_RESULT_UNKNOWN：原请求 metadata-request-1 尚未核对。"};
  const retry=vi.fn(async()=>{metadata={status:"saved"};notifyNoteReviewSaveState();});let stop!:()=>void;
  await act(async()=>{stop=registerNoteReviewStateContributor("page-1",{source:"metadata",getState:()=>metadata,retry});});
  cleanups.push(async()=>stop());
  expect(h.get().saveStatus).toBe("failed");expect(h.get().saveMessage).toContain(request.id);expect(h.get().saveMessage).toContain("metadata-request-1");
  expect(h.get().saveMessage).toContain("正文：");expect(h.get().saveMessage).toContain("页面及关联：");
  await act(async()=>h.get().retrySave());expect(retry).toHaveBeenCalledOnce();expect(h.get().saveStatus).toBe("saved");
});

it("allows close only after current H/B′ are durably preserved, including a failed authority save", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  const latest = snapshot("page-1", 3, "Latest unsaved input").document;
  fixture.gateway.saveDocument.mockRejectedValueOnce(new Error("authority offline"));
  await act(async () => h.get().onDocumentChange(latest));
  await act(async () => { expect(await h.get().ensureDurable()).toBe(true); });
  expect(fixture.preserved.get("page-1")).toMatchObject({content:latest});
  expect(fixture.options.onError).not.toHaveBeenCalled();
  fixture.hbsGateway.savePreservation = vi.fn(async () => { throw new Error("恢复空间不足"); });
  await act(async () => { expect(await h.get().ensureDurable()).toBe(false); });
  expect(fixture.options.onError).toHaveBeenLastCalledWith(expect.stringContaining("恢复空间不足"));
  expect(fixture.options.onError.mock.lastCall?.[0]).not.toContain("[object Object]");
});
it("restores old local text as a conflict and does not autosave it over a later database revision", async () => {
  const fixture = pageFixture(); f.loadNote.mockReturnValue({ document: snapshot("page-1", 1, "Draft").document, baseRevision: 1 });
  const h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  expect(h.get().saveStatus).toBe("failed");
  await act(async () => { expect(await h.get().ensureDurable()).toBe(true); });
  expect(fixture.gateway.saveDocument).not.toHaveBeenCalled(); expect(fixture.preserved.get("page-1")).toMatchObject({content:snapshot("page-1",1,"Draft").document});
});
it("blocks closing when the IndexedDB recovery receipt fails even though its staged draft reads back", async () => {
  const previousStorage = getAppStorage(), database = createMemoryPreviewDatabase();
  configurePreviewDatabase(database.open);
  const storage = await createIndexedPreviewStorage("note-close-indexeddb:"); selectAppStorage(storage);
  const fixture = pageFixture(); f.loadNote.mockReturnValue({document: snapshot("page-1", 1, "Draft").document, baseRevision: 1});
  const h = await mount(() => useNoteDocumentSession(fixture.options));
  cleanups.push(async () => { selectAppStorage(previousStorage); });
  await act(async () => h.get().loadPage(notePageId("page-1")));
  fixture.hbsGateway.savePreservation = vi.fn(async (_kind, _id, checkpoint) => {
    storage.setItem("recovery", JSON.stringify(checkpoint)); await storagePersistence(storage)!.flush(); return {revision:1};
  });
  database.failNext(new DOMException("full", "QuotaExceededError"));
  await act(async () => expect(await h.get().ensureDurable()).toBe(false));
  expect(JSON.parse(storage.getItem("recovery")!)).toMatchObject({content:snapshot("page-1",1,"Draft").document});
  expect(fixture.options.onError).toHaveBeenCalledWith(expect.stringContaining("QuotaExceededError"));
  await storagePersistence(storage)!.retry();
  await act(async () => expect(await h.get().ensureDurable()).toBe(true));
});
it("reuses a prefetched document through the shared recovery and save owner", async () => {
  const fixture = pageFixture();
  f.loadNote.mockReturnValue({ document: snapshot("page-1", 1, "Draft").document, baseRevision: 1 });
  const h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1"), true, snapshot()));
  expect(fixture.gateway.loadDocument).not.toHaveBeenCalled();
  expect(h.get().snapshot?.document).toEqual(snapshot("page-1", 1, "Draft").document);
  expect(h.get().saveStatus).toBe("failed");
  expect(h.get().loadState).toEqual({ phase: "ready", pageId: "page-1" });
  await act(async () => { expect(await h.get().ensureDurable()).toBe(true); });
  expect(fixture.gateway.saveDocument).not.toHaveBeenCalled();
  expect(fixture.preserved.get("page-1")).toMatchObject({content:snapshot("page-1",1,"Draft").document});
});
it("ignores older page loads, late save errors and changes from an old editor", async () => {
  const fixture = pageFixture(), pending = deferred<NoteDocumentSnapshot>(); fixture.gateway.loadDocument.mockReturnValueOnce(pending.promise);
  const h = await mount(() => useNoteDocumentSession(fixture.options));
  let first!: Promise<void>;
  await act(async () => { first = h.get().loadPage(notePageId("page-1")); });
  await act(async () => h.get().loadPage(notePageId("page-2")));
  await act(async () => { pending.resolve(snapshot()); await first; });
  expect(h.get().snapshot?.page.id).toBe("page-2"); expect(fixture.options.onLoaded).toHaveBeenCalledOnce();
  await act(async () => h.get().onDocumentChange(snapshot().document));
  expect(f.saveNote).not.toHaveBeenCalled();
  const save = deferred<NoteDocumentSnapshot>(); fixture.gateway.saveDocument.mockReturnValueOnce(save.promise);
  let flushing!: Promise<void>;
  await act(async () => { h.get().onDocumentChange(snapshot("page-2", 3, "Changed").document); flushing = h.get().saveControllerRef.current!.flush(); });
  await act(async () => h.get().loadPage(notePageId("page-3")));
  await act(async () => { save.reject(new Error("old save failed")); await flushing; });
  expect(h.get().saveStatus).toBe("saved"); expect(h.get().snapshot?.page.id).toBe("page-3");
});
it("retains an unknown acceptance lease on close and refuses close if recovery storage fails", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  await act(async () => { h.get().saveControllerRef.current?.beginExternalChange(3, snapshot().document)?.markDispatched(); });
  await act(async () => { expect(await h.get().ensureDurable()).toBe(true); });
  expect(fixture.options.onNotice).toHaveBeenLastCalledWith(expect.stringContaining("持久保全"));
  expect(h.get().saveControllerRef.current?.hasExternalChange()).toBe(true);
  fixture.hbsGateway.savePreservation = vi.fn(async () => { throw new Error("quota"); });
  await act(async () => { expect(await h.get().ensureDurable()).toBe(false); });
});

it("ignores a superseded load failure and refuses a mismatched document response", async () => {
  const fixture = pageFixture(), pending = deferred<NoteDocumentSnapshot>(); fixture.gateway.loadDocument.mockReturnValueOnce(pending.promise);
  const h = await mount(() => useNoteDocumentSession(fixture.options)); let first!: Promise<void>;
  await act(async () => { first = h.get().loadPage(notePageId("page-1")); });
  await act(async () => h.get().loadPage(notePageId("page-2")));
  await act(async () => { pending.reject(new Error("old request")); await expect(first).resolves.toBeUndefined(); });
  fixture.gateway.loadDocument.mockResolvedValueOnce(snapshot("wrong"));
  await act(async () => { await expect(h.get().loadPage(notePageId("page-3"))).rejects.toThrow("身份不匹配"); });
  expect(h.get().snapshot?.page.id).toBe("page-2");
});

it("refreshes changed structure but preserves input typed while its response is pending", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  const pending = deferred<NoteDocumentSnapshot>(); fixture.gateway.loadDocument.mockReturnValueOnce(pending.promise);
  const session = h.get().editorSessionId;
  let refresh!: Promise<void>;
  await act(async () => { refresh = h.get().refreshAfterStructure(notePageId("page-1")); });
  const latest = snapshot("page-1", 3, "New input").document;
  await act(async () => h.get().onDocumentChange(latest));
  await act(async () => { pending.resolve(snapshot("page-1", 4, "Server structure")); await refresh; });
  expect(h.get().snapshot?.document).toEqual(snapshot("page-1",4,"Server structure").document);
  expect(fixture.documents.get("page-1")?.baseline).toEqual(latest);
  expect(h.get().editorSessionId).toBe(session);
  const hbs = h.get().retainedSessions.find(value=>value.snapshot.page.id==="page-1")?.hbs!;
  expect(hbs.getSnapshot().conflicts).toHaveLength(0); expect(hbs.getSnapshot().baselineProjection).toEqual(latest);
  await act(async () => h.get().ensureDurable());
  expect(fixture.preserved.get("page-1")).toMatchObject({content:snapshot("page-1",4,"Server structure").document});
  await act(async () => { hbs.resolveMany(hbs.getSnapshot().changes.map(change=>change.id),"undo"); await h.get().ensureDurable(); });
  expect(h.get().snapshot?.document).toEqual(latest);
});

it("blocks close only for an active Agent command and resumes after release", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  holdTextEditor("page-1", "agent-command", true);
  try {
    await act(async () => expect(await h.get().ensureDurable()).toBe(false));
    expect(f.saveNote).not.toHaveBeenCalled();
    expect(fixture.options.onError).toHaveBeenCalledWith(expect.stringContaining("agent正在写入"));
  } finally { holdTextEditor("page-1", "agent-command", false); }
  await act(async () => expect(await h.get().ensureDurable()).toBe(true));
});

it("restores legacy review D/B and blocks new requests until its exact original request is settled", async () => {
  const fixture = pageFixture(), latest = snapshot("page-1", 1, "Latest review input").document;
  f.loadNote.mockReturnValue({ document: latest, baseRevision: 1, review: { document: latest, baseline: snapshot().document, request: { requestId: "original-request", pageId: "page-1", action: "edit", document: latest } } });
  const h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  expect(h.get().snapshot?.document).toEqual(latest);
  expect(h.get().saveControllerRef.current?.hasUnsavedChanges()).toBe(true);
  const lease = h.get().saveControllerRef.current?.beginExternalChange(3, latest);
  expect(lease).toBeNull();
  expect(h.get().saveMessage).toContain("original-request");
  expect(fixture.gateway.saveDocument).not.toHaveBeenCalled();
});

it("does not clear complete review recovery when an older formal receipt is acknowledged", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  const state: NoteReviewSaveState = { status: "saving" };
  const unregister = registerNoteReviewSavePort("page-1", { getState: () => state, retry: vi.fn(), flush: vi.fn() });
  cleanups.push(async () => unregister());
  const lease = h.get().saveControllerRef.current?.beginExternalChange(3, snapshot().document);
  expect(lease?.markDispatched()).toBe(true);
  await act(async () => {
    fixture.documents.set("page-1", {version:4,saved:snapshot("page-1",4,"Earlier acknowledged input").document,baseline:snapshot().document});
    await h.get().saveControllerRef.current?.synchronizeExternal?.(snapshot("page-1",4,"Earlier acknowledged input"));
    expect(lease?.commitGroup(snapshot("page-1", 4, "Earlier acknowledged input"), 4, () => undefined)).toBe(true);
  });
  expect(f.clearNote).not.toHaveBeenCalled();
});

it("does not authorize a destructive operation on a merely local recovery draft", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  await act(async () => h.get().saveControllerRef.current?.restore(snapshot("page-1",3,"Recovery").document,"conflict"));
  await act(async () => { expect(await h.get().ensureDurable(true)).toBe(false); });
  expect(f.saveNote).not.toHaveBeenCalled();
  await act(async () => { expect(await h.get().ensureDurable()).toBe(true); });
  expect(fixture.preserved.get("page-1")).toMatchObject({content:snapshot("page-1",3,"Recovery").document});
});

it("retains the existing save owner, editor and recovery when replacement loading fails", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  const original = h.get().saveControllerRef.current, session = h.get().editorSessionId;
  fixture.gateway.loadDocument.mockRejectedValueOnce(new Error("disk unavailable"));
  await act(async () => { await expect(h.get().loadPage(notePageId("page-2"))).rejects.toThrow("disk unavailable"); });
  expect(h.get().saveControllerRef.current).toBe(original);
  expect(h.get().snapshot?.page.id).toBe("page-1"); expect(h.get().editorSessionId).toBe(session);
  expect(f.clearNote).not.toHaveBeenCalled();
});

it("reactivates an open page with the same save owner and editor without reading its body or links again", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  const owner = h.get().saveControllerRef.current, editor = h.get().editorSessionId;
  await act(async () => h.get().loadPage(notePageId("page-2")));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  expect(h.get().saveControllerRef.current).toBe(owner);
  expect(h.get().editorSessionId).toBe(editor);
  expect(fixture.gateway.loadDocument).toHaveBeenCalledTimes(2);
  expect(fixture.gateway.listOutgoing).toHaveBeenCalledTimes(2);
  expect(fixture.gateway.listBacklinks).toHaveBeenCalledTimes(2);
});

it("saves a hidden editor through its own controller without replacing the active page", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  const editor = h.get().editorSessionId;
  await act(async () => h.get().loadPage(notePageId("page-2")));
  const changed = snapshot("page-1", 3, "Background input").document;
  await act(async () => {
    h.get().onRetainedDocumentChange(changed, editor);
    expect(await h.get().ensureDurable(false, notePageId("page-1"))).toBe(true);
  });
  expect(h.get().snapshot?.page.id).toBe("page-2");
  expect(fixture.gateway.saveDocument).toHaveBeenCalledWith(expect.objectContaining({document: changed}));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  expect(h.get().snapshot?.document).toEqual(changed);
  expect(h.get().editorSessionId).toBe(editor);
});

it("keeps open tabs across Home but disposes closed tabs and reads them afresh on reopening", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  const owner = h.get().saveControllerRef.current!, editor = h.get().editorSessionId, dispose = vi.spyOn(owner, "dispose");
  await act(async () => h.get().deactivate());
  await act(async () => h.get().loadPage(notePageId("page-1")));
  expect(h.get().saveControllerRef.current).toBe(owner);
  expect(fixture.gateway.loadDocument).toHaveBeenCalledOnce();
  await act(async () => h.get().loadPage(notePageId("page-2")));
  await act(async () => h.get().retain(["page-2"]));
  expect(dispose).toHaveBeenCalledOnce();
  await act(async () => h.get().loadPage(notePageId("page-1")));
  expect(fixture.gateway.loadDocument).toHaveBeenCalledTimes(3);
  expect(h.get().editorSessionId).not.toBe(editor);
  await act(async () => h.get().onRetainedDocumentChange(snapshot("page-1", 3, "Stale editor").document, editor));
  expect(h.get().snapshot?.document).toEqual(snapshot().document);
});

it("invalidates a retained page only after observing a newer resource revision", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  await act(async () => h.get().loadPage(notePageId("page-2")));
  h.get().observePages([snapshot("page-1", 2).page]);
  await act(async () => h.get().loadPage(notePageId("page-1")));
  expect(fixture.gateway.loadDocument).toHaveBeenCalledTimes(2);
  await act(async () => h.get().loadPage(notePageId("page-2")));
  h.get().observePages([snapshot("page-1", 4).page]);
  fixture.gateway.loadDocument.mockResolvedValueOnce(snapshot("page-1", 4, "External update"));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  expect(fixture.gateway.loadDocument).toHaveBeenCalledTimes(3);
  expect(h.get().snapshot?.document).toEqual(snapshot("page-1", 4, "External update").document);
});

it("keeps the editor when its own save receipt catches up with an earlier index notification", async () => {
  const fixture = pageFixture(), h = await mount(() => useNoteDocumentSession(fixture.options));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  const owner = h.get().saveControllerRef.current!, editor = h.get().editorSessionId;
  const lease = owner.beginExternalChange(3, snapshot().document)!;
  expect(lease.markDispatched()).toBe(true);
  h.get().observePages([snapshot("page-1", 4).page]);
  await act(async () => {
    fixture.documents.set("page-1",{version:4,saved:snapshot().document,baseline:snapshot().document});
    await owner.synchronizeExternal?.(snapshot("page-1",4));
    expect(lease.commitGroup(snapshot("page-1", 4), 4, () => undefined)).toBe(true);
  });
  await act(async () => h.get().loadPage(notePageId("page-2")));
  await act(async () => h.get().loadPage(notePageId("page-1")));
  expect(h.get().saveControllerRef.current).toBe(owner); expect(h.get().editorSessionId).toBe(editor);
  expect(fixture.gateway.loadDocument).toHaveBeenCalledTimes(2);
});

it("coalesces tab and navigation activation without rebuilding the current editor", async () => {
  const fixture = pageFixture(), pending = deferred<NoteDocumentSnapshot>();
  fixture.gateway.loadDocument.mockReturnValueOnce(pending.promise);
  const h = await mount(() => useNoteDocumentSession(fixture.options));
  let first!: Promise<void>, duplicate!: Promise<void>;
  await act(async () => {
    first = h.get().loadPage(notePageId("page-1"));
    duplicate = h.get().loadPage(notePageId("page-1"));
  });
  expect(first).toBe(duplicate);
  await act(async () => { pending.resolve(snapshot()); await first; });
  const owner = h.get().saveControllerRef.current, editor = h.get().editorSessionId;
  await act(async () => h.get().loadPage(notePageId("page-1")));
  expect(fixture.gateway.loadDocument).toHaveBeenCalledOnce();
  expect(h.get().saveControllerRef.current).toBe(owner); expect(h.get().editorSessionId).toBe(editor);
  await act(async () => h.get().loadPage(notePageId("page-1"), true, undefined, true));
  expect(fixture.gateway.loadDocument).toHaveBeenCalledTimes(2);
  expect(h.get().saveControllerRef.current).not.toBe(owner);
});

it("restores a durable HBS owner when retained React effects restart",async()=>{
  const fixture=pageFixture(),host=document.createElement("div"),root=createRoot(host);let current!:ReturnType<typeof useNoteDocumentSession>;
  function Harness(){current=useNoteDocumentSession(fixture.options);return null;}
  const render=async(mode:"visible"|"hidden")=>{await act(async()=>root.render(<Activity mode={mode}><Harness/></Activity>));};
  cleanups.push(async()=>{await act(async()=>root.unmount());host.remove();});
  await render("visible");await act(async()=>current.loadPage(notePageId("page-1")));
  await act(async()=>{current.onDocumentChange(snapshot("page-1",3,"durable input").document);await current.ensureDurable();});
  await render("hidden");await render("visible");
  expect(current.retainedSessions).toHaveLength(1);expect(current.saveControllerRef.current).toBeDefined();
  expect(current.snapshot?.document).toEqual(snapshot("page-1",4,"durable input").document);
});

it("preserves unsaved input in every retained page before restarting active and hidden owners",async()=>{
  const fixture=pageFixture(),host=document.createElement("div"),root=createRoot(host);let current!:ReturnType<typeof useNoteDocumentSession>;
  function Harness(){current=useNoteDocumentSession(fixture.options);return null;}
  const render=async(mode:"visible"|"hidden")=>{await act(async()=>root.render(<Activity mode={mode}><Harness/></Activity>));};
  cleanups.push(async()=>{await act(async()=>root.unmount());host.remove();});
  await render("visible");await act(async()=>current.loadPage(notePageId("page-1")));
  const hiddenInput=snapshot("page-1",3,"unsaved hidden input").document;
  await act(async()=>current.onDocumentChange(hiddenInput));
  const original=current.saveControllerRef.current!;
  await act(async()=>current.loadPage(notePageId("page-2")));
  const activeInput=snapshot("page-2",3,"unsaved active input").document;
  await act(async()=>current.onDocumentChange(activeInput));
  expect(fixture.gateway.saveDocument).not.toHaveBeenCalled();
  await render("hidden");
  expect(fixture.preserved.get("page-1")).toMatchObject({content:hiddenInput});
  expect(fixture.preserved.get("page-2")).toMatchObject({content:activeInput});
  await render("visible");
  expect(current.snapshot?.page.id).toBe("page-2");expect(current.snapshot?.document).toEqual(activeInput);
  expect(current.retainedSessions).toHaveLength(2);
  expect(current.retainedSessions.find(session=>session.snapshot.page.id==="page-1")?.snapshot.document).toEqual(hiddenInput);
  expect(fixture.gateway.saveDocument).not.toHaveBeenCalled();
  await act(async()=>current.loadPage(notePageId("page-1")));
  expect(current.snapshot?.document).toEqual(hiddenInput);expect(current.saveControllerRef.current).not.toBe(original);
});
