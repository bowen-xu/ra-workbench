// @vitest-environment happy-dom
import {beforeEach, expect, it, vi} from "vitest";
import type {NoteDocumentSnapshot} from "@research-assistant/data/notes";
import {selectAppStorage} from "../../../core/runtime/appStorage";
import {createMemoryNotesDatabase} from "../../../core/runtime/testing/memoryNotesDatabase";
import {configureBrowserNotesDatabase} from "../../../core/features/notes/gateways/browserNotesDatabase";
import {browserNotesGateway as notes} from "../../../core/features/notes/gateways/browserNotesGateway";
import {readState, writeState} from "../../../core/features/notes/gateways/browserNotesStorage";
import {createBrowserNoteHbsAuthority} from "../../../core/features/notes/gateways/browserNoteHbsAuthority";
import {NoteSaveController} from "../../../core/features/notes/editor/noteSaveController";
import {registerTextInstallPort, type AppliedNotePage} from "../../../core/features/notes/editor/noteTextEditorPorts";
import {MockNoteTextSession} from "./MockNoteTextSession";
import {createMockTextPatchGateway, TEXT_PATCH_PREVIEW_CONVERSATION, TEXT_PATCH_PREVIEW_PAGE} from "./mockTextPatchGateway";
import type {TextPatchDecision, TextPatchResolution} from "../../../core/features/agent/textPatchGateway";

let memory = createMemoryNotesDatabase(), failCommit = false;
beforeEach(() => {
  localStorage.clear(); selectAppStorage(localStorage); memory = createMemoryNotesDatabase(); failCommit = false;
  configureBrowserNotesDatabase(async name => {
    const db = await memory.open(name);
    return {...db, commit: async (...args) => {if (failCommit) throw new Error("transaction rolled back"); return db.commit(...args);}};
  });
});
async function fixture() {
  const patch = createMockTextPatchGateway(true), original = patch.snapshot().document;
  const created = await notes.createPage({title: "Legacy HBS integration"}), state = await readState();
  await writeState({...state, pages: [{...created.page, id: TEXT_PATCH_PREVIEW_PAGE}], documents: {[TEXT_PATCH_PREVIEW_PAGE]: original}});
  let current: NoteDocumentSnapshot = await notes.loadDocument(TEXT_PATCH_PREVIEW_PAGE), live = current.document;
  const authority = createBrowserNoteHbsAuthority();
  const save = vi.fn((document: typeof original, expectedRevision: number, review?: AppliedNotePage["review"]) =>
    authority.saveExternal({document, expectedRevision, saveReason: "structure"}, review?.baseline ?? document));
  const session = new MockNoteTextSession(notes, TEXT_PATCH_PREVIEW_PAGE, undefined, save);
  const controller = new NoteSaveController({pageId: TEXT_PATCH_PREVIEW_PAGE, gateway: notes, initialRevision: 0,
    onSnapshot: snapshot => {current = snapshot;}, onStatus: () => {}});
  const release = session.register({controller, gateway: notes, getSnapshot: () => current, onChanged: () => {}, onError: () => {}});
  const unregister = registerTextInstallPort(TEXT_PATCH_PREVIEW_PAGE, {isInstalling: false, read: () => live,
    prepare: (_before, after) => () => {live = after; return undefined;},
    prepareStructure: (_before, after) => () => {live = after; return undefined;}, dispose() {}});
  patch.connectHost(session);
  // Initialize through the host before planning edits: Notes supplies the
  // formal title as part of its document, including for this legacy fixture.
  const initialWrites = memory.metrics.documentWrites;
  await patch.prepare();
  expect(patch.snapshot()).toMatchObject({document: current.document, revision: current.page.revision});
  expect(await authority.read(TEXT_PATCH_PREVIEW_PAGE)).toEqual({version: current.page.revision, saved: current.document, baseline: current.document});
  expect(memory.metrics.documentWrites).toBe(initialWrites); expect(save).not.toHaveBeenCalled();
  let closed = false;
  const close = () => {if (!closed) {closed = true; unregister(); release(); controller.dispose();}};
  const resolve = (action: TextPatchResolution["action"], extra: Partial<TextPatchResolution> = {}): TextPatchResolution => {
    const review = patch.snapshot().review!;
    return {conversationId: TEXT_PATCH_PREVIEW_CONVERSATION, proposalId: review.proposalId, generation: review.generation,
      pageId: TEXT_PATCH_PREVIEW_PAGE, requestId: crypto.randomUUID(), action, ...extra};
  };
  const decide = (action: TextPatchDecision["action"]): TextPatchDecision => {
    const review = patch.snapshot().review!;
    return {conversationId: TEXT_PATCH_PREVIEW_CONVERSATION, proposalId: review.proposalId, generation: review.generation,
      requestId: crypto.randomUUID(), action};
  };
  return {patch, authority, original, save, close, resolve, decide};
}

it("commits mounted legacy Keep and its B-only undo/redo through the HBS repository without rewriting S", async () => {
  const f = await fixture();
  try {
    await f.patch.simulate("rewrite");
    const pending = await f.authority.read(TEXT_PATCH_PREVIEW_PAGE), writes = memory.metrics.documentWrites;
    const frozen = f.patch.snapshot().review!, keep = f.resolve("keep");
    const kept = await f.patch.resolve(keep);
    expect(await f.authority.read(TEXT_PATCH_PREVIEW_PAGE)).toEqual({version: pending.version + 1, saved: pending.saved, baseline: pending.saved});
    expect(kept.pages).toMatchObject([{revision: pending.version + 1, document: pending.saved}]);
    expect(kept.review?.pages[0]?.expectedRevision).toBe(pending.version + 1);
    const undo = await f.patch.resolve(f.resolve("restore", {historyRequestId: keep.requestId, direction: "undo"}));
    expect(await f.authority.read(TEXT_PATCH_PREVIEW_PAGE)).toEqual({version: pending.version + 2, saved: pending.saved, baseline: pending.baseline});
    expect(undo.review?.state).toBe("pending");
    await f.patch.resolve(f.resolve("restore", {historyRequestId: keep.requestId, direction: "redo"}));
    expect(await f.authority.read(TEXT_PATCH_PREVIEW_PAGE)).toEqual({version: pending.version + 3, saved: pending.saved, baseline: pending.saved});
    expect(memory.metrics.documentWrites).toBe(writes);
    const calls = f.save.mock.calls.length;
    expect(await f.patch.resolve(keep)).toBe(kept); expect(f.save).toHaveBeenCalledTimes(calls);
    const history = await f.patch.review(TEXT_PATCH_PREVIEW_CONVERSATION, frozen.proposalId, frozen.generation);
    expect(history?.pages[0]?.afterDocument).toEqual(frozen.pages[0]?.afterDocument);
  } finally {f.close();}
});

it("accepts a closed note with a B-only CAS and rolls back a failed original request before retry", async () => {
  const f = await fixture();
  try {
    await f.patch.simulate("rewrite"); f.close();
    const before = await f.authority.read(TEXT_PATCH_PREVIEW_PAGE), writes = memory.metrics.documentWrites;
    const request = f.decide("accept");
    failCommit = true;
    await expect(f.patch.decide(request)).rejects.toThrow("transaction rolled back");
    expect(await f.authority.read(TEXT_PATCH_PREVIEW_PAGE)).toEqual(before);
    expect(f.patch.snapshot().review?.state).toBe("pending");
    failCommit = false;
    await f.patch.decide(request); await f.patch.decide(request);
    expect(await f.authority.read(TEXT_PATCH_PREVIEW_PAGE)).toEqual({version: before.version + 1, saved: before.saved, baseline: before.saved});
    expect(f.patch.snapshot().review?.state).toBe("accept"); expect(memory.metrics.documentWrites).toBe(writes);
    expect(f.save.mock.calls.at(-1)?.[2]).toMatchObject({baseline: before.saved, requestId: request.requestId});
  } finally {f.close();}
});

it("does not turn an unmounted body edit into an unfenced write", async () => {
  const f = await fixture(); f.close();
  const before = await f.authority.read(TEXT_PATCH_PREVIEW_PAGE);
  await expect(f.patch.simulate("rewrite")).rejects.toMatchObject({status: "failed"});
  expect(await f.authority.read(TEXT_PATCH_PREVIEW_PAGE)).toEqual(before); expect(f.save).not.toHaveBeenCalled();
});
