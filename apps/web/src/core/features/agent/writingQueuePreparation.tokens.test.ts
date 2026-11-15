import {afterEach, expect, it, vi} from "vitest";
import {decodeAgentWritingQueuePreviewRequest} from "@research-assistant/core/agent";
import {registerStoragePersistence} from "../../runtime/storagePersistence";
import {selectAppStorage} from "../../runtime/appStorage";
import {ConversationDraftStore, conversationDraftKey} from "./conversationDraftStore";
import {localConversationDraftPersistence} from "./conversationDraftPersistence";
import {writingDraftAcknowledgments} from "./writingDraftAcknowledgment";
import {WritingQueuePreparation} from "./writingQueuePreparation";
import {WritingDiffDemoStorage} from "../../../app/previews/agent/WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "../../../app/previews/agent/WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "../../../app/previews/agent/writingDiffDemoRuntime";
class MemoryStorage implements Storage {
  readonly rows = new Map<string, string>(); get length() {return this.rows.size;}
  key(index: number) {return [...this.rows.keys()][index] ?? null;}
  getItem(key: string) {return this.rows.get(key) ?? null;} setItem(key: string, value: string) {this.rows.set(key, value);}
  removeItem(key: string) {this.rows.delete(key);} clear() {this.rows.clear();}
}
const cleanups: (() => void)[] = [];
afterEach(() => {for (const cleanup of cleanups.splice(0)) cleanup();});
async function fixture() {
  const storage = new MemoryStorage(), draftStorage = new MemoryStorage(); selectAppStorage(storage);
  const scope = {manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID()}, inputId = crypto.randomUUID();
  const files = new WritingDiffDemoStorage(scope.manuscriptId, []), agent = new WritingDiffDemoAgentStorage(files, undefined);
  const runtime = createWritingDiffDemoRuntime(files, agent, scope.manuscriptId, scope.conversationId);
  cleanups.push(() => runtime.dispose());
  const added = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: 0, edit: {type: "add", inputId, text: "original", userInput: []}});
  const intent = decodeAgentWritingQueuePreviewRequest({inputId, expectedRevision: added.revision, submission: {
    conversationId: scope.conversationId, requestId: crypto.randomUUID(), text: "original", context: {manuscriptId: scope.manuscriptId, files: []},
    resources: [{kind: "manuscript", resourceId: scope.manuscriptId, parts: [], groupAccess: "readOnly", newMemberAccess: "readOnly"}]}});
  const drafts = new ConversationDraftStore(localConversationDraftPersistence(draftStorage));
  const draftKey = conversationDraftKey({hostKind: "manuscript", hostId: scope.manuscriptId, conversationId: scope.conversationId});
  drafts.restoreQueue(draftKey, {input: {inputId, text: "original"}, index: 0, preceding: [], following: []});
  const token = drafts.captureAcknowledgmentToken(draftKey, "original")!, actual = writingDraftAcknowledgments(drafts);
  const port = {flush: vi.fn(actual.flush), retry: vi.fn(actual.retry), consume: vi.fn(actual.consume)};
  const open = (configure = true) => {const owner = new WritingQueuePreparation(runtime.queue.preview, storage, () => {}, scope);
    if (configure) owner.configureDraftAcknowledgments(port); cleanups.push(() => owner.dispose()); return owner;};
  const prepare = vi.spyOn(runtime.queue.preview, "prepare"), status = vi.spyOn(runtime.queue.preview, "prepareStatus"), start = vi.spyOn(runtime.queue.preview, "start");
  const locator = `research-assistant:writing-queue-prepare:v1:${scope.manuscriptId}:${scope.conversationId}`;
  return {storage, draftStorage, scope, intent, drafts, draftKey, token, port, open, prepare, status, start, locator};
}
it("flushes the complete token envelope before the draft port and the first prepare RPC", async () => {
  const f = await fixture(); let release: (() => void) | undefined;
  f.port.flush.mockImplementationOnce(() => new Promise(resolve => {release = resolve;}));
  const owner = f.open(), work = owner.begin(f.intent, [], f.token);
  await vi.waitFor(() => expect(release).toBeTypeOf("function"));
  expect(JSON.parse(f.storage.getItem(f.locator)!)).toMatchObject({schemaVersion: 2, intent: f.intent, draftAcknowledgment: f.token, workspaceReady: false});
  expect(f.prepare).not.toHaveBeenCalled(); release!(); await work;
  expect(f.prepare).toHaveBeenCalledExactlyOnceWith(f.intent); expect(owner.getSnapshot()).toMatchObject({phase: "ready", knownCommitted: true, draftAcknowledgment: f.token, workspaceReady: false});
  expect(owner.canStart()).toBe(false); expect(f.port.consume).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
});
it("a missing or failed draft port preserves the locator and cannot prepare; retry uses the same original token", async () => {
  const f = await fixture(), owner = f.open(false); await owner.begin(f.intent, [], f.token);
  expect(owner.getSnapshot().phase).toBe("localError"); expect(f.prepare).not.toHaveBeenCalled();
  f.port.flush.mockRejectedValueOnce(new Error("draft flush unavailable")); owner.configureDraftAcknowledgments(f.port); await owner.retry();
  expect(owner.getSnapshot().phase).toBe("localError"); expect(f.prepare).not.toHaveBeenCalled(); expect(f.status).not.toHaveBeenCalled();
  await owner.retry(); expect(f.port.retry).toHaveBeenCalledTimes(2); expect(f.prepare).toHaveBeenCalledExactlyOnceWith(f.intent);
  expect(owner.getSnapshot().draftAcknowledgment).toEqual(f.token); expect(f.drafts.read(f.draftKey)).toBe("original");
});
it("retains a confirmed token preparation across the workspace-to-start cold gap and requires fresh workspace confirmation", async () => {
  const f = await fixture(), owner = f.open(); await owner.begin(f.intent, [], f.token); await owner.finishWorkspace(f.intent.submission.requestId);
  expect(owner.canStart()).toBe(true); expect(owner.retainsLocator()).toBe(true);
  expect(JSON.parse(f.storage.getItem(f.locator)!)).toMatchObject({workspaceReady: true, draftAcknowledgment: f.token}); owner.dispose();
  const reopened = f.open(); expect(reopened.getSnapshot().workspaceReady).toBe(false); expect(reopened.canStart()).toBe(false);
  await reopened.check(); expect(reopened.getSnapshot()).toMatchObject({phase: "ready", draftAcknowledgment: f.token, workspaceReady: false}); expect(reopened.canStart()).toBe(false);
  await reopened.finishWorkspace(f.intent.submission.requestId); expect(reopened.canStart()).toBe(true); expect(reopened.retainsLocator()).toBe(true);
  expect(f.prepare).toHaveBeenCalledOnce(); expect(f.start).not.toHaveBeenCalled(); expect(f.port.consume).not.toHaveBeenCalled();
  await reopened.retireStarted({inputId: f.intent.inputId, expectedRevision: 0, start: {conversationId: f.scope.conversationId, manuscriptId: f.scope.manuscriptId,
    requestId: f.intent.submission.requestId, contextSnapshotId: reopened.getSnapshot().result!.preview.contextSnapshotId, model: reopened.getSnapshot().result!.preview.input.model, mode: "deepSeekNonThinking"}});
  expect(f.storage.getItem(f.locator)).toBeNull(); expect(f.drafts.read(f.draftKey)).toBe("original");
});
it("does not grant canStart until workspace-ready evidence has reliably flushed", async () => {
  const f = await fixture(), owner = f.open(); await owner.begin(f.intent, [], f.token);
  let release: (() => void) | undefined;
  registerStoragePersistence(f.storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush: () => new Promise<void>(resolve => {release = resolve;}), retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
  const finish = owner.finishWorkspace(f.intent.submission.requestId); await vi.waitFor(() => expect(release).toBeTypeOf("function"));
  expect(owner.getSnapshot()).toMatchObject({phase: "persisting", workspaceReady: true}); expect(owner.canStart()).toBe(false);
  release!(); await finish; expect(owner.canStart()).toBe(true); expect(owner.retainsLocator()).toBe(true); expect(f.start).not.toHaveBeenCalled();
});
it("waits for an in-flight workspace save before actually retiring its original known-start locator", async () => {
  const f = await fixture(), owner = f.open(); await owner.begin(f.intent, [], f.token);
  let release: (() => void) | undefined, writes = 0;
  registerStoragePersistence(f.storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush: async () => {if (++writes === 1) await new Promise<void>(resolve => {release = resolve;});},
    retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
  const finish = owner.finishWorkspace(f.intent.submission.requestId); await vi.waitFor(() => expect(release).toBeTypeOf("function"));
  const retired = owner.retireStarted({inputId: f.intent.inputId, expectedRevision: 0, start: {conversationId: f.scope.conversationId, manuscriptId: f.scope.manuscriptId,
    requestId: f.intent.submission.requestId, contextSnapshotId: owner.getSnapshot().result!.preview.contextSnapshotId, model: owner.getSnapshot().result!.preview.input.model, mode: "deepSeekNonThinking"}});
  expect(owner.retainsLocator()).toBe(true); release!(); await Promise.all([finish, retired]);
  expect(owner.retainsLocator()).toBe(false); expect(f.storage.getItem(f.locator)).toBeNull(); expect(owner.getSnapshot().phase).toBe("retired");
  expect(f.prepare).toHaveBeenCalledOnce(); expect(f.port.consume).not.toHaveBeenCalled();
});
it("disposal makes a late retirement inert while an earlier workspace flush is still pending", async () => {
  const f = await fixture(), owner = f.open(); await owner.begin(f.intent, [], f.token);
  let release: (() => void) | undefined;
  registerStoragePersistence(f.storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush: () => new Promise<void>(resolve => {release = resolve;}), retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
  const finish = owner.finishWorkspace(f.intent.submission.requestId); await vi.waitFor(() => expect(release).toBeTypeOf("function")); owner.dispose();
  await owner.retireStarted({inputId: f.intent.inputId, expectedRevision: 0, start: {conversationId: f.scope.conversationId, manuscriptId: f.scope.manuscriptId,
    requestId: f.intent.submission.requestId, contextSnapshotId: owner.getSnapshot().result!.preview.contextSnapshotId, model: owner.getSnapshot().result!.preview.input.model, mode: "deepSeekNonThinking"}});
  expect(f.storage.getItem(f.locator)).not.toBeNull(); release!(); await finish; expect(f.storage.getItem(f.locator)).not.toBeNull(); expect(f.start).not.toHaveBeenCalled();
});
it("v1 remains unowned and cannot start before a real workspace confirmation; its old cleanup remains compatible", async () => {
  const f = await fixture(); f.storage.setItem(f.locator, JSON.stringify({schemaVersion: 1, intent: f.intent, definitiveRejection: false, rejectionCause: null, knownCommitted: false, workspaceBuffers: []}));
  const owner = f.open(); expect(owner.getSnapshot().draftAcknowledgment).toBeNull(); expect(owner.canStart()).toBe(false);
  await owner.retry(); expect(owner.getSnapshot().phase).toBe("ready"); expect(owner.canStart()).toBe(false);
  await owner.finishWorkspace(f.intent.submission.requestId); expect(owner.getSnapshot().phase).toBe("complete"); expect(owner.canStart()).toBe(true);
  expect(f.storage.getItem(f.locator)).toBeNull(); expect(f.port.consume).not.toHaveBeenCalled(); expect(f.drafts.read(f.draftKey)).toBe("original");
});
it("does not dispatch even a prepare when the reliable token-envelope save fails", async () => {
  const f = await fixture(); registerStoragePersistence(f.storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush: async () => {throw new Error("locator flush unavailable");}, retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
  const owner = f.open(); await owner.begin(f.intent, [], f.token);
  expect(owner.getSnapshot().phase).toBe("localError"); expect(f.port.flush).not.toHaveBeenCalled(); expect(f.prepare).not.toHaveBeenCalled();
});
it("rejects foreign draft provenance and incomplete v2 envelopes on cold restoration", async () => {
  const f = await fixture();
  for (const value of [
    {schemaVersion: 2, intent: f.intent, draftAcknowledgment: {...f.token, originKey: JSON.stringify(["manuscript", crypto.randomUUID(), f.scope.conversationId])}, workspaceReady: false},
    {schemaVersion: 2, intent: f.intent, draftAcknowledgment: f.token},
  ]) {
    f.storage.setItem(f.locator, JSON.stringify(value)); const owner = f.open(); expect(owner.getSnapshot().phase).toBe("localError"); await owner.retry();
    expect(f.prepare).not.toHaveBeenCalled(); expect(f.status).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
  }
});
it("retireStarted refuses a same-R preparation with a different token, including a cold owner's newer token", async () => {
  const f = await fixture(), owner = f.open(); await owner.begin(f.intent, [], f.token);
  const preview = owner.getSnapshot().result!.preview;
  const start = {inputId: f.intent.inputId, expectedRevision: 0, start: {conversationId: f.scope.conversationId, manuscriptId: f.scope.manuscriptId,
    requestId: f.intent.submission.requestId, contextSnapshotId: preview.contextSnapshotId, model: preview.input.model, mode: "deepSeekNonThinking" as const}};
  const newerToken = {...f.token, revision: crypto.randomUUID()};
  const newer = JSON.stringify({...JSON.parse(f.storage.getItem(f.locator)!), draftAcknowledgment: newerToken}); f.storage.setItem(f.locator, newer);
  await owner.retireStarted(start); expect(owner.getSnapshot().phase).toBe("localError"); expect(f.storage.getItem(f.locator)).toBe(newer); owner.dispose();
  const reopened = f.open(); expect(reopened.getSnapshot().draftAcknowledgment).toEqual(newerToken);
  await reopened.retireStarted(start, f.token); expect(reopened.getSnapshot().phase).toBe("localError"); expect(f.storage.getItem(f.locator)).toBe(newer);
  expect(reopened.getSnapshot().draftAcknowledgment).toEqual(newerToken); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.status).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
});
it("workspace cleanup and its removed-flush retry preserve a same-R locator with a newly owned token", async () => {
  const f = await fixture(), owner = f.open(); await owner.begin(f.intent, []);
  const original = JSON.parse(f.storage.getItem(f.locator)!); let failing = true;
  registerStoragePersistence(f.storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush: async () => {if (failing) throw new Error("old removal not flushed");}, retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
  await owner.finishWorkspace(f.intent.submission.requestId); expect(f.storage.getItem(f.locator)).toBeNull();
  const newer = JSON.stringify({...original, draftAcknowledgment: f.token, workspaceReady: false}); f.storage.setItem(f.locator, newer); failing = false;
  await owner.check(); expect(owner.getSnapshot().phase).toBe("localError"); expect(f.storage.getItem(f.locator)).toBe(newer);
  expect(f.prepare).toHaveBeenCalledOnce(); expect(f.status).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
});
