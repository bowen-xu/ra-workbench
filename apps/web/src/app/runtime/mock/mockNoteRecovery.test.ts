// @vitest-environment happy-dom
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {createAppRuntime} from "../createAppRuntime";
import {getAppRuntime, selectAppRuntime} from "../../../core/runtime/appRuntime";
import {getNotesGateway} from "../../../core/features/notes/gateways";
import {AppNoteHbsSession} from "../../../core/features/notes/workspace/appNoteHbsSession";
import {getHbsDocumentGateway, getPendingHbsDocumentCommit} from "../../../core/services/hbsDocumentGateway";
import {notePageId} from "@research-assistant/core/notes";
import {readNoteHbsState} from "../../../core/features/notes/gateways/browserNotesStorage";
import type {NoteRecoveryScenario} from "../../../core/runtime/noteRecoveryPreview";
import {flushStorage} from "../../../core/runtime/storagePersistence";

const owners: AppNoteHbsSession[] = [];
beforeEach(() => {
  localStorage.clear(); localStorage.setItem("business-marker", "untouched");
  selectAppRuntime({mode: "browser", storage: localStorage, gateways: {}, dispose() {}});
  vi.stubGlobal("fetch", vi.fn(async () => new Response("%PDF-1.4\nsynthetic sample")));
});
afterEach(() => {owners.splice(0).forEach(owner => owner.dispose()); getAppRuntime().dispose(); vi.unstubAllGlobals();});
async function fixture(scenario: NoteRecoveryScenario) {
  const runtime = await createAppRuntime(`?preview=app&note-recovery=${scenario}`, localStorage);
  runtime.preview!.configure({delayMs: 0, nextFailure: null});
  const pageId = notePageId(runtime.noteRecovery!.pageId), notes = getNotesGateway();
  const onStatus = vi.fn();
  const open = async () => {
    const owner = await AppNoteHbsSession.open({loaded: await notes.loadDocument(pageId), onStatus, onSnapshot: () => {}, onDurable: () => {}});
    owners.push(owner); owner.start(); return owner;
  };
  expect(localStorage.getItem("business-marker")).toBe("untouched");
  return {runtime, pageId, notes, onStatus, open};
}
it.each(["conflict", "resolution-failure", "resolution-unknown", "stale-resolution"] as const)("%s reconstructs a genuine HBS content conflict", async scenario => {
  const f = await fixture(scenario), owner = await f.open(), state = owner.session.getSnapshot();
  expect(state.conflicts).toHaveLength(1); expect(f.onStatus.mock.lastCall?.[0]).toBe("conflict");
  expect(JSON.stringify(state.content)).toContain("185%"); expect(JSON.stringify(state.observed.saved)).toContain("90%");
  expect(JSON.stringify(state.observed.baseline)).toContain("80%");
  if (scenario === "conflict") return;
  const pages = (await f.notes.listPages()).length;
  owner.session.resolve(state.conflicts[0]!.id, "keep");
  if (scenario === "stale-resolution") {
    await vi.waitFor(() => expect(owner.session.getSnapshot().conflicts.length).toBeGreaterThan(0));
    expect(owner.session.getSnapshot().observed.version).toBeGreaterThan(state.observed.version);
    expect(JSON.stringify(owner.session.getSnapshot().observed.saved)).toContain("又更新了正文");
    expect(JSON.stringify(owner.session.getSnapshot().content)).toContain("185%");
    expect(getPendingHbsDocumentCommit("note", f.pageId)).toBeUndefined();
  } else {
    await vi.waitFor(() => expect(f.onStatus.mock.lastCall?.[1]).toContain("MOCK_NOTE_RESOLUTION_"));
    const original = getPendingHbsDocumentCommit("note", f.pageId)!;
    expect(original).toBeDefined();
    const saved = JSON.stringify((await f.notes.loadDocument(f.pageId)).document);
    expect(saved).toContain(scenario === "resolution-failure" ? "90%" : "185%");
    await owner.retry();
    expect(getPendingHbsDocumentCommit("note", f.pageId)).toEqual(original);
    await f.runtime.noteRecovery!.releaseFault(); await owner.retry();
    expect(getPendingHbsDocumentCommit("note", f.pageId)).toBeUndefined();
    expect(owner.session.getSnapshot().pending).toBe(false);
    expect(JSON.stringify((await f.notes.loadDocument(f.pageId)).document)).toContain("185%");
    expect((await f.notes.listPages()).length).toBe(pages);
  }

});
it.each(["save-failure", "lost-receipt", "stuck-request"] as const)("%s persists and replays the exact original request", async scenario => {
  const f = await fixture(scenario), owner = await f.open();
  const original = getPendingHbsDocumentCommit("note", f.pageId)!;
  expect(original).toBeDefined(); expect(f.onStatus.mock.lastCall?.[0]).toBe("failed");
  expect(f.onStatus.mock.lastCall?.[1]).toContain(original.id);
  const state = await getHbsDocumentGateway().read("note", f.pageId);
  expect(JSON.stringify(state)).toContain(scenario === "save-failure" ? "80%" : "81%");
  if (scenario !== "lost-receipt") {
    await expect(owner.retry()).rejects.toThrow("MOCK_NOTE_");
    expect(getPendingHbsDocumentCommit("note", f.pageId)).toEqual(original);
    await f.runtime.noteRecovery!.releaseFault();
  }
  await owner.retry();
  expect(getPendingHbsDocumentCommit("note", f.pageId)).toBeUndefined();
  expect(f.onStatus.mock.lastCall?.[0]).toBe("saved");
  expect(JSON.stringify(owner.session.getSnapshot().content)).toContain("81%");
});
it.each(["checkpoint-read", "checkpoint-invalid"] as const)("%s preserves the actual durable checkpoint while its read response is faulty", async scenario => {
  const f = await fixture(scenario), owner = await f.open();
  expect(owner.session.getSnapshot().preservationError).toContain("MODEL_PRESERVATION_FAILED");
  await expect(owner.ensurePreserved()).rejects.toThrow("MODEL_PRESERVATION_FAILED");
  const stored = (await readNoteHbsState(f.pageId)).hbsRecords?.[f.pageId]?.checkpoint;
  expect(JSON.stringify(stored?.content)).toContain("81%");
  owner.dispose(); await f.runtime.noteRecovery!.releaseFault();
  const reopened = await f.open();
  expect(reopened.session.getSnapshot().preservationError).toBeUndefined();
  expect(JSON.stringify(reopened.session.getSnapshot().content)).toContain("81%");
});
it("checkpoint-write fails real preservation before any formal document commit", async () => {
  const f = await fixture("checkpoint-write"), owner = await f.open();
  await expect(owner.flush()).rejects.toThrow("MOCK_NOTE_CHECKPOINT_WRITE");
  expect(getPendingHbsDocumentCommit("note", f.pageId)).toBeUndefined();
  expect(JSON.stringify((await f.notes.loadDocument(f.pageId)).document)).toContain("80%");
  expect(JSON.stringify(owner.session.getSnapshot().content)).toContain("81%");
  await f.runtime.noteRecovery!.releaseFault(); await owner.retry();
  expect(JSON.stringify((await f.notes.loadDocument(f.pageId)).document)).toContain("81%");
});

it("reopening the same preview retains the fixture identity and exact unresolved request", async () => {
  const f = await fixture("lost-receipt"), request = getPendingHbsDocumentCommit("note", f.pageId);
  const count = (await f.notes.listPages()).length;
  f.runtime.dispose();
  const reopened = await createAppRuntime("?preview=app&note-recovery=lost-receipt", localStorage);
  expect(reopened.noteRecovery!.pageId).toBe(f.pageId);
  expect(getPendingHbsDocumentCommit("note", f.pageId)).toEqual(request);
  expect((await getNotesGateway().listPages()).length).toBe(count);
  const saved = await getNotesGateway().loadDocument(f.pageId);
  expect(saved.document.blocks[1]!.content.kind).toBe("asset");
});

it("fault release and scene changes leave the ordinary app preview space intact", async () => {
  const ordinary = await createAppRuntime("?preview=app", localStorage);
  ordinary.storage.setItem("ordinary-preview-marker", "retained"); await flushStorage(ordinary.storage); ordinary.dispose();
  const fault = await fixture("checkpoint-invalid");
  expect(fault.runtime.storage.getItem("ordinary-preview-marker")).toBeNull();
  await fault.runtime.noteRecovery!.releaseFault(); fault.runtime.dispose();
  const other = await fixture("stuck-request");
  expect(other.pageId).not.toBe(fault.pageId); other.runtime.dispose();
  const restored = await createAppRuntime("?preview=app", localStorage);
  expect(restored.storage.getItem("ordinary-preview-marker")).toBe("retained");
  expect(restored.noteRecovery).toBeUndefined();
});
