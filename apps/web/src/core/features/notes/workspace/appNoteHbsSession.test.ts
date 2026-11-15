// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type NoteDocumentV1, type NoteIndexItem } from "@research-assistant/core/notes";
import type { DocumentCommit } from "@research-assistant/core/hbs";
import type { NoteDocumentSnapshot, NotesGateway } from "@research-assistant/data/notes";
import type { AppHbsDocumentGateway } from "../../../services/hbsDocumentGateway";
import { acknowledgeNoteReviewRequest, noteReviewRecovery, synchronizeNoteReview, registerTextInstallPort } from "../editor/noteTextEditorPorts";
import type { NoteEditorInstallPort } from "../editor/noteEditorInstallPort";
import { AppNoteHbsSession } from "./appNoteHbsSession";
import { NotesClient } from "../NotesClient";
import { applyFavoriteAction } from "../../../../../../../plugins/note_manager/src/features/notes/workspace/noteFavoriteActions";
import { getReaderCloseCoordinator } from "../../../workspace/DocumentCloseCoordinator";
import {getAppStorage} from "../../../runtime/appStorage";

afterEach(() => vi.useRealTimers());
const pageId = notePageId("note-hbs-test");
function document(text: string): NoteDocumentV1 {
  return { schemaVersion: 1, pageId, blocks: [{ id: noteBlockId("paragraph"), type: "paragraph", props: {}, children: [],
    content: { kind: "inline", items: [{ type: "text", text, styles: [] }] } }] };
}
function fixture(saved = document("80%"), baseline = saved) {
  let authority = { version: 3, saved, baseline }, checkpoint: unknown = null;
  const commits: DocumentCommit<unknown>[] = [];
  const loaded: NoteDocumentSnapshot = { page: { id: pageId, schemaVersion: 1, title: "Note", revision: 3, position: 0,
    status: "active", isFavorite: false, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now" }, document: saved };
  const gateway: AppHbsDocumentGateway = {
    read: vi.fn(async () => structuredClone(authority)),
    commit: vi.fn(async (_kind, _id, request) => { commits.push(request);
      if (request.expectedVersion !== authority.version) return { type: "conflict", requestId: request.id, snapshot: authority };
      authority = { version: authority.version + 1, saved: request.saved as NoteDocumentV1, baseline: request.baseline as NoteDocumentV1 };
      return { type: "receipt", requestId: request.id, snapshot: authority };
    }),
    readPreservation: vi.fn(async () => checkpoint), savePreservation: vi.fn(async (_kind, _id, value) => { checkpoint = structuredClone(value); return { revision: 1 }; }),
  };
  return { loaded, gateway, commits, get: () => authority, preserved: () => checkpoint,
    external: (next: NoteDocumentV1) => { authority = { ...authority, version: authority.version + 1, saved: next }; } };
}
async function open(f: ReturnType<typeof fixture>) {
  const onStatus = vi.fn(), onSnapshot = vi.fn();
  const controller = await AppNoteHbsSession.open({ loaded: f.loaded, gateway: f.gateway, onStatus, onSnapshot, onDurable: vi.fn() });
  controller.start(); return { controller, onStatus, onSnapshot };
}

it("refreshes related page metadata only after the entrance change is acknowledged", async () => {
  const target = "entrance-target", entrance: NoteDocumentV1 = {...document(""), blocks: [{
    id: noteBlockId("entry"), type: "researchObjectReference", props: {pageEntry: true}, children: [],
    content: {kind: "reference", targetKind: "note_page", targetId: target, labelSnapshot: "Page", quoteSnapshot: ""},
  }]};
  const f = fixture(entrance), onRelatedChange = vi.fn();
  const controller = await AppNoteHbsSession.open({loaded: f.loaded, gateway: f.gateway,
    onStatus: vi.fn(), onSnapshot: vi.fn(), onDurable: vi.fn(), onRelatedChange});
  controller.start();
  try {
    controller.enqueue(document("cut"));
    expect(onRelatedChange).not.toHaveBeenCalled();
    await controller.flush();
    expect(onRelatedChange).toHaveBeenCalledExactlyOnceWith([target]);
    controller.enqueue(entrance);
    expect(onRelatedChange).toHaveBeenCalledTimes(1);
    await controller.flush();
    expect(onRelatedChange).toHaveBeenCalledTimes(2);
    expect(onRelatedChange.mock.lastCall?.[0]).toEqual([target]);
  } finally {controller.dispose();}
});

it("autosaves ordinary edits through the HBS port and durably preserves S′", async () => {
  const f = fixture(), { controller } = await open(f);
  vi.useFakeTimers();
  controller.session.edit(document("ordinary input"));
  await vi.advanceTimersByTimeAsync(2999); expect(f.commits).toHaveLength(0);
  await vi.advanceTimersByTimeAsync(1);
  expect(f.commits).toHaveLength(1);
  expect(f.get().saved).toEqual(document("ordinary input")); expect(f.get().baseline).toEqual(document("ordinary input"));
  expect(f.preserved()).toMatchObject({ content: document("ordinary input") }); expect(controller.hasUnsavedChanges()).toBe(false);
  controller.dispose();
});

it("does not rewrite an unchanged saved document during repeated switch guards", async () => {
  const f = fixture(), {controller} = await open(f);
  vi.mocked(f.gateway.savePreservation).mockClear();
  try {
    await controller.flush(); await controller.flush();
    expect(f.gateway.savePreservation).not.toHaveBeenCalled();
    expect(f.commits).toHaveLength(0);
  } finally {controller.dispose();}
});
it("unlocks a reopened review only after its exact verified receipt, preserving newer input",async()=>{
 const f=fixture(),request={conversationId:"conversation",proposalId:"proposal",generation:1,requestId:"cache-recovery",pageId,action:"retain-current" as const,document:f.loaded.document,expectedRevision:3};
 const controller=await AppNoteHbsSession.open({loaded:f.loaded,gateway:f.gateway,onStatus:vi.fn(),onSnapshot:vi.fn(),onDurable:vi.fn(),
  recovery:{baseRevision:3,updatedAt:"now",document:f.loaded.document,review:{document:f.loaded.document,baseline:f.loaded.document,request}}});controller.start();
 try{
  controller.enqueue(document("Newer local input"));await controller.flush();expect(f.commits).toHaveLength(0);
  acknowledgeNoteReviewRequest(pageId,"unrelated");await controller.flush();expect(f.commits).toHaveLength(0);
  acknowledgeNoteReviewRequest(pageId,request.requestId);await controller.flush();
  expect(f.get().saved).toEqual(document("Newer local input"));expect(controller.isEditingBlocked()).toBe(false);
  expect(noteReviewRecovery(pageId)).toBeUndefined();
 }finally{controller.dispose();}
});

it("preserves an inactive restored candidate without attempting a body commit or repeated checkpoint writes", async () => {
  const f = fixture(document("85%"), document("80%")), first = await open(f);
  first.controller.enqueue(document("185%")); await first.controller.ensurePreserved(); first.controller.dispose();
  const {controller} = await open({...f, loaded: {...f.loaded, page: {...f.loaded.page, status: "trashed"}}});
  try {
    await controller.flush(); const writes = vi.mocked(f.gateway.savePreservation).mock.calls.length;
    await controller.flush(); await controller.ensurePreserved();
    expect(f.commits).toHaveLength(0);
    expect(controller.session.getSnapshot().content).toEqual(document("185%"));
    expect(f.gateway.savePreservation).toHaveBeenCalledTimes(writes);
  } finally {controller.dispose();}
});

it("saving edited candidate keeps B pending and a later S edit conflicts", async () => {
  const f = fixture(document("85%"), document("80%")), { controller } = await open(f);
  controller.session.edit(document("185%")); await controller.flush();
  expect(f.get().saved).toEqual(document("185%")); expect(f.get().baseline).toEqual(document("80%"));
  f.external(document("90%")); await synchronizeNoteReview(pageId);
  expect(controller.session.getSnapshot().content).toEqual(document("185%"));
  expect(controller.session.getSnapshot().conflicts).toHaveLength(1);
  await controller.ensurePreserved(); controller.dispose();
  const reopened = await open({ ...f, loaded: { ...f.loaded, page: { ...f.loaded.page, revision: f.get().version }, document: f.get().saved } });
  expect(reopened.controller.session.getSnapshot().content).toEqual(document("185%"));
  expect(reopened.controller.session.getSnapshot().conflicts).toHaveLength(1); reopened.controller.dispose();
});

it("close preservation and ordinary flush never retry an unknown request", async () => {
  const f = fixture(); let calls = 0;
  const original = f.gateway.commit;
  f.gateway.commit = async (...args) => { calls++; if (calls === 1) throw new Error("transport disconnected"); return original(...args); };
  const { controller } = await open(f); controller.enqueue(document("local input")); await controller.flush();
  expect(calls).toBe(1); expect(controller.hasUnsavedChanges()).toBe(true);
  controller.enqueue(document("newer input after unknown outcome"));
  await controller.ensurePreserved(); await controller.flush(); expect(calls).toBe(1);
  await controller.retry(); expect(calls).toBeGreaterThanOrEqual(2); expect(controller.hasUnsavedChanges()).toBe(false);
  expect(f.get().saved).toEqual(document("newer input after unknown outcome")); controller.dispose();
});

it("exposes retry for a recovered unresolved image save instead of an unresolvable comparison", async () => {
  const image = (width: number): NoteDocumentV1 => ({schemaVersion: 1, pageId, blocks: [{
    id: noteBlockId("image"), type: "image", props: {assetId: "fixture", previewWidth: width},
    content: {kind: "none"}, children: [],
  }]});
  const f = fixture(image(494)), storage = getAppStorage(), key = `research-assistant:hbs-outbox:note:${pageId}`;
  const request = {id: "recovered-image:1", expectedVersion: 2, saved: image(494), baseline: image(502)};
  storage.setItem(key, JSON.stringify(request));
  const original = f.gateway.commit;
  f.gateway.commit = vi.fn(async (...args: Parameters<AppHbsDocumentGateway["commit"]>) => {
    const reply = await original(...args); storage.removeItem(key); return reply;
  });
  const {controller, onStatus} = await open(f);
  try {
    expect(onStatus.mock.lastCall?.[0]).toBe("failed");
    expect(onStatus.mock.lastCall?.[1]).toContain("MODEL_REQUEST_UNSETTLED");
    expect(onStatus.mock.lastCall?.[1]).toContain(request.id);
    await controller.flush(); expect(f.gateway.commit).not.toHaveBeenCalled();
    await controller.retry();
    expect(f.gateway.commit).toHaveBeenCalledWith("note", pageId, request);
    expect(storage.getItem(key)).toBeNull();
    expect(onStatus.mock.lastCall?.[0]).toBe("saved");
    expect(controller.session.getSnapshot().content).toEqual(image(494));
  } finally {controller.dispose(); storage.removeItem(key);}
});

it("Agent receipts reconcile through the same session without ordinary history or a second write", async () => {
  const f = fixture(), { controller } = await open(f); let visible = f.loaded.document;
  const install = vi.fn((_before, next, _review, options) => () => { expect(options).toMatchObject({ addToHistory: false }); visible = next; return undefined; });
  const unregister = registerTextInstallPort(pageId, { read: () => visible, prepareStructure: install } as unknown as NoteEditorInstallPort);
  try {
    const lease = controller.beginExternalChange(3, f.loaded.document)!; expect(lease.markDispatched()).toBe(true);
    f.external(document("incoming candidate"));
    const receipt = { ...f.loaded, page: { ...f.loaded.page, revision: 4 }, document: f.get().saved };
    await controller.synchronizeExternal(receipt);
    expect(lease.commitGroup(receipt, 4, () => undefined)).toBe(true);
    expect(controller.session.getSnapshot().content).toEqual(receipt.document);
    expect(controller.session.getSnapshot().baselineProjection).toEqual(document("80%"));
    expect(install).toHaveBeenCalledOnce(); expect(f.commits).toHaveLength(0);
  } finally { unregister(); controller.dispose(); }
});

it("does not overwrite an unreadable existing S′ with the authoritative fallback", async () => {
  const f = fixture(); f.gateway.readPreservation = vi.fn(async () => ({ format: 1, kind: "note", content: "damaged" }));
  const { controller } = await open(f);
  expect(controller.session.getSnapshot().preservationError).toContain("MODEL_PRESERVATION_FAILED");
  controller.enqueue(document("new local input"));
  await expect(controller.ensurePreserved()).rejects.toThrow("MODEL_PRESERVATION_FAILED");
  expect(f.gateway.savePreservation).not.toHaveBeenCalled(); expect(f.commits).toHaveLength(0); controller.dispose();
});

it("conservatively retains legacy review provenance when a human candidate was already saved", async () => {
  const f = fixture(document("185%"), document("80%"));
  const controller = await AppNoteHbsSession.open({ loaded: f.loaded, gateway: f.gateway,
    recovery: { document: document("185%"), baseRevision: 3, updatedAt: "now", review: { document: document("185%"), baseline: document("80%") } },
    onStatus: vi.fn(), onSnapshot: vi.fn(), onDurable: vi.fn() });
  controller.start(); f.external(document("90%")); await synchronizeNoteReview(pageId);
  expect(controller.session.getSnapshot().content).toEqual(document("185%")); expect(controller.session.getSnapshot().conflicts).toHaveLength(1);
  controller.dispose();
});

it("reports the immutable HBS request ID while its receipt is pending, separately from S′ preservation",async()=>{
  const f=fixture(),original=f.gateway.commit;let release!:()=>void;
  const held=new Promise<void>(resolve=>{release=resolve;});
  f.gateway.commit=vi.fn(async(...args:Parameters<AppHbsDocumentGateway["commit"]>)=>{await held;return original(...args);});
  const {controller,onStatus}=await open(f);controller.enqueue(document("pending input"));const flushing=controller.flush();
  for(let n=0;n<30;n++)await Promise.resolve();
  const request=vi.mocked(f.gateway.commit).mock.calls[0]![2];
  expect(onStatus.mock.lastCall?.[0]).toBe("saving");expect(onStatus.mock.lastCall?.[1]).toContain(request.id);
  expect(onStatus.mock.lastCall?.[1]).toContain("MODEL_DOCUMENT_SAVING");expect(f.preserved()).toMatchObject({content:document("pending input")});
  release();await flushing;expect(onStatus.mock.lastCall?.[0]).toBe("saved");controller.dispose();
});

it("finishes delete and subsequent navigation guards when HBS flush republishes the host snapshot", async () => {
  const f = fixture(), coordinator = getReaderCloseCoordinator();
  let unregister = () => {}, controller: AppNoteHbsSession;
  const prepare = vi.fn(async () => {
    // Fail deterministically if a live Map iteration starts chasing render
    // registrations, instead of hanging the regression runner indefinitely.
    if (prepare.mock.calls.length > 3) throw new Error("Repeated preparation of the same note owner");
    await controller.flush();
    return true;
  });
  const onSnapshot = vi.fn(() => {
    unregister();
    unregister = coordinator.register("note-delete-hbs-owner", { isActive: () => false, prepare, commit: () => {} });
  });
  controller = await AppNoteHbsSession.open({ loaded: f.loaded, gateway: f.gateway, onSnapshot, onStatus: vi.fn(), onDurable: vi.fn() });
  controller.start();
  let current = f.loaded.page;
  const apply = vi.fn<NotesGateway["applyMetadataBatch"]>(async request => {
    expect(prepare).toHaveBeenCalledOnce();
    expect(f.preserved()).toMatchObject({ content: f.loaded.document });
    current = { ...current, status: "trashed", revision: current.revision + 1 };
    f.external(f.get().saved);
    return { operationId: request.operationId, succeededIds: [pageId], failures: [], pages: [current], bodyChangedIds: [], favoritePageIds: [] };
  });
  const client = new NotesClient({ listPageMetadata: async () => [current], applyMetadataBatch: apply } as unknown as NotesGateway);
  const item: NoteIndexItem = { id: pageId, title: current.title, snippet: "", updatedAt: current.updatedAt,
    category: "independent", status: "active", isFavorite: false, target: { kind: "page", pageId } };
  try {
    await applyFavoriteAction(client, item, "trash");
    expect(apply).toHaveBeenCalledOnce();
    expect(current.status).toBe("trashed");
    expect(onSnapshot.mock.calls.length).toBeGreaterThan(1);
    expect(controller.session.getSnapshot().conflicts).toHaveLength(0);
    expect(await coordinator.prepareAll()).toBe(true);
    expect(prepare).toHaveBeenCalledTimes(2);
  } finally { unregister(); controller.dispose(); }
});

it("retains full metadata receipts without changing pending review and rejects stale or unrelated metadata", async () => {
  const f = fixture(document("85%"), document("80%")), { controller, onSnapshot } = await open(f);
  const initial = controller.session.getSnapshot();
  const review = { content: initial.content, baseline: initial.baselineProjection, changes: initial.changes };
  let current = f.loaded.page;
  try {
    for (const status of ["trashed", "active"] as const) {
      f.external(f.get().saved);
      current = { ...current, revision: f.get().version, status, title: `Updated ${status}`, isFavorite: true };
      controller.updateMetadata(current);
      await synchronizeNoteReview(pageId);
      await controller.flush();
      expect(onSnapshot.mock.lastCall?.[0].page).toEqual(current);
      const state = controller.session.getSnapshot();
      expect({ content: state.content, baseline: state.baselineProjection, changes: state.changes }).toEqual(review);
      expect(f.commits).toHaveLength(0);
    }
    controller.updateMetadata({ ...f.loaded.page, title: "Stale metadata" });
    expect(() => controller.updateMetadata({ ...current, id: notePageId("another-note") })).toThrow("MODEL_IDENTITY_MISMATCH");
    await controller.flush();
    expect(onSnapshot.mock.lastCall?.[0].page).toEqual(current);
    // The authority can be newer than the latest metadata receipt. Its version
    // is also a lower bound, so an intermediate metadata reply cannot regress.
    f.external(f.get().saved);
    await synchronizeNoteReview(pageId);
    controller.updateMetadata({ ...current, title: "Older than HBS" });
    await controller.flush();
    expect(onSnapshot.mock.lastCall?.[0].page).toEqual({ ...current, revision: f.get().version });
  } finally { controller.dispose(); }
});
