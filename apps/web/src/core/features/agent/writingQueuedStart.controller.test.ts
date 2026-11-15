import {afterEach, expect, it, vi} from "vitest";
import * as CoreAgent from "@research-assistant/core/agent";
import type {AgentWritingQueueCancelRequest, AgentWritingQueueCancelStatusResponse} from "@research-assistant/core/agent";
import {registerStoragePersistence} from "../../runtime/storagePersistence";
import {WritingSubmissionController, acquireWritingSubmissionController, releaseWritingSubmissionController} from "./writingSubmissionController";
import {WritingDiffDemoStorage} from "../../../app/previews/agent/WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "../../../app/previews/agent/WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "../../../app/previews/agent/writingDiffDemoRuntime";
import {ConversationDraftStore, conversationDraftKey} from "./conversationDraftStore";
import {localConversationDraftPersistence} from "./conversationDraftPersistence";
import {writingDraftAcknowledgments, type WritingDraftAcknowledgmentPort} from "./writingDraftAcknowledgment";

class MemoryStorage implements Storage {
  readonly rows = new Map<string, string>();
  get length() {return this.rows.size;}
  key(index: number) {return [...this.rows.keys()][index] ?? null;}
  getItem(key: string) {return this.rows.get(key) ?? null;}
  setItem(key: string, value: string) {this.rows.set(key, value);}
  removeItem(key: string) {this.rows.delete(key);}
  clear() {this.rows.clear();}
}
const cleanups: (() => void)[] = [];
afterEach(() => {cleanups.splice(0).forEach(cleanup => cleanup()); vi.restoreAllMocks();});
function persistence(storage: Storage, flush: () => Promise<void>) {
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush, retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
}
async function fixture(autoStart = true, withToken = false) {
  const scope = {conversationId: crypto.randomUUID(), manuscriptId: crypto.randomUUID()}, storage = new MemoryStorage();
  const files = new WritingDiffDemoStorage(scope.manuscriptId, []), agent = new WritingDiffDemoAgentStorage(files, undefined);
  const runtime = createWritingDiffDemoRuntime(files, agent, scope.manuscriptId, scope.conversationId); runtime.execution.historyOnly = true;
  const owners: WritingSubmissionController[] = [];
  let draftPort: WritingDraftAcknowledgmentPort | undefined;
  const open = () => {const owner = new WritingSubmissionController(runtime.gateways.submission, runtime.gateways.turns,
    runtime.queue.preview, {scope, storage}); if (draftPort) owner.configureDraftAcknowledgments(draftPort); owners.push(owner); return owner;};
  cleanups.push(() => {owners.forEach(owner => owner.dispose()); runtime.dispose();});
  const inputId = crypto.randomUUID(), tailId = crypto.randomUUID();
  let queue = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: 0,
    edit: {type: "add", inputId, text: "original request"}});
  queue = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: queue.revision,
    edit: {type: "add", inputId: tailId, text: "tail preserved"}});
  const request = {conversationId: scope.conversationId, requestId: crypto.randomUUID(), text: "original request", context: {manuscriptId: scope.manuscriptId, files: []}};
  const draftStorage = new MemoryStorage(), drafts = new ConversationDraftStore(localConversationDraftPersistence(draftStorage));
  const draftKey = conversationDraftKey({hostKind: "manuscript", hostId: scope.manuscriptId, conversationId: scope.conversationId});
  if (withToken) {drafts.restoreQueue(draftKey, {input: {inputId, text: request.text}, index: 0, preceding: [], following: []}); draftPort = writingDraftAcknowledgments(drafts);}
  const token = withToken ? drafts.captureAcknowledgmentToken(draftKey, request.text) : null;
  const owner = open(); await owner.prepareQueued(request, inputId, queue.revision, undefined, [], token);
  const preview = owner.getSnapshot().preview!;
  const prepareKey = [...storage.rows.keys()].find(key => key.includes("queue-prepare"))!, prepareEnvelope = storage.getItem(prepareKey)!;
  await runtime.gateways.workspace.prepare({scope: {...scope, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId}, buffers: []});
  await owner.observeWorkspacePreparation(preview.requestId, true);
  const startOriginal = runtime.queue.preview.start, statusOriginal = runtime.queue.preview.startStatus;
  const start = vi.spyOn(runtime.queue.preview, "start"), status = vi.spyOn(runtime.queue.preview, "startStatus");
  const prepare = vi.spyOn(runtime.queue.preview, "prepare"), prepareStatus = vi.spyOn(runtime.queue.preview, "prepareStatus"), recover = vi.spyOn(runtime.queue.preview, "recover");
  const cancel = vi.spyOn(runtime.queue.preview, "cancel"), cancelStatus = vi.spyOn(runtime.queue.preview, "cancelStatus"), admitted = vi.fn();
  const intent = CoreAgent.decodeAgentWritingQueuedStartRequest({inputId, expectedRevision: owner.getSnapshot().queue!.revision,
    start: {...scope, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId, model: preview.input.model, mode: "deepSeekNonThinking"}});
  if (autoStart) {runtime.queue.failNextStart("afterStarted"); await owner.confirm("deepSeekNonThinking", admitted);}
  const startKey = `research-assistant:writing-queue-start:v1:${scope.manuscriptId}:${scope.conversationId}`;
  const cancelKey = `research-assistant:writing-queue-cancel:v1:${scope.manuscriptId}:${scope.conversationId}`;
  const cancelIntent: AgentWritingQueueCancelRequest = {...scope, inputId, requestId: crypto.randomUUID(), previewRequestId: preview.requestId, expectedRevision: intent.expectedRevision};
  const seedRelated = (requestId = preview.requestId) => {
    const preparation = JSON.parse(prepareEnvelope); preparation.intent.submission.requestId = requestId;
    storage.setItem(prepareKey, JSON.stringify(preparation));
    storage.setItem(cancelKey, JSON.stringify({schemaVersion: 1, intent: {...cancelIntent, previewRequestId: requestId}, definitiveConflict: false}));
  };
  const resetReads = () => {status.mockClear(); recover.mockClear(); prepareStatus.mockClear(); cancelStatus.mockClear();};
  const reopen = () => {owner.dispose(); resetReads(); return open();};
  const assertNoMutations = () => {expect(start).toHaveBeenCalledOnce(); expect(prepare).not.toHaveBeenCalled(); expect(cancel).not.toHaveBeenCalled();};
  return {scope, storage, runtime, owner, open, reopen, preview, tailId, intent, startKey, prepareKey, cancelKey, cancelIntent,
    start, status, startOriginal, statusOriginal, prepare, prepareStatus, recover, cancel, cancelStatus, admitted, seedRelated, assertNoMutations,
    drafts, draftStorage, draftKey, token};
}

it("reopens an actual lost Started ACK read-only and delivers the original admission before blocked related cleanup", async () => {
  const f = await fixture();
  expect(f.owner.getSnapshot()).toMatchObject({error: "start", startRecovery: {phase: "unknown", intent: f.intent}});
  expect(JSON.parse(f.storage.getItem(f.startKey)!).intent).toEqual(f.intent); expect(f.admitted).not.toHaveBeenCalled();
  f.seedRelated(); const reopened = f.reopen(), observed = vi.fn();
  expect(f.status).not.toHaveBeenCalled(); expect(f.cancelStatus).not.toHaveBeenCalled(); f.assertNoMutations();
  const withoutWorkspace = f.runtime.execution.snapshot().map(([key, value]) => {
    const {workspace: _workspace, buffers: _buffers, ...record} = value; return [key, record] as const;
  });
  f.runtime.execution.restore(withoutWorkspace); vi.spyOn(CoreAgent, "executionModeForModel").mockReturnValue(null);
  const workspace = vi.spyOn(f.runtime.gateways.workspace, "restore").mockRejectedValue(new Error("must not resolve workspace"));
  const validate = vi.spyOn(f.runtime.execution, "validateQueuedStart");
  let release!: () => void; const blocked = new Promise<void>(resolve => {release = resolve;});
  persistence(f.storage, async () => {if (!f.storage.getItem(f.prepareKey) && f.storage.getItem(f.cancelKey)) await blocked;});
  const work = reopened.checkQueuedStart(observed);
  await vi.waitFor(() => expect(observed).toHaveBeenCalledOnce());
  expect(observed).toHaveBeenCalledWith({scope: {...f.scope, requestId: f.intent.start.requestId, contextSnapshotId: f.intent.start.contextSnapshotId}, turnId: f.preview.turnId});
  expect(reopened.getSnapshot()).toMatchObject({phase: "started", startRecovery: {phase: "clearing", intent: f.intent, result: {alreadyStarted: true}}});
  expect(f.storage.getItem(f.startKey)).not.toBeNull(); release(); await work;
  expect(reopened.getSnapshot().startRecovery?.phase).toBe("done"); expect(observed).toHaveBeenCalledOnce(); expect(f.storage.length).toBe(0);
  expect(workspace).not.toHaveBeenCalled(); expect(validate).not.toHaveBeenCalled(); f.assertNoMutations();
});
it("a true Started fact retires three matching locators using only read-only cancellation NotFound", async () => {
  const f = await fixture(); f.seedRelated(); const queue = await f.runtime.queue.gateway.read(f.scope), reopened = f.reopen(), observed = vi.fn();
  expect(f.storage.length).toBe(3); expect(f.status).not.toHaveBeenCalled(); expect(f.cancelStatus).not.toHaveBeenCalled();
  await reopened.checkQueuedStart(observed);
  expect(f.status).toHaveBeenCalledExactlyOnceWith(f.intent); expect(f.cancelStatus).toHaveBeenCalledExactlyOnceWith(f.cancelIntent);
  expect(f.prepareStatus).not.toHaveBeenCalled(); expect(f.recover).not.toHaveBeenCalled(); f.assertNoMutations();
  expect(f.storage.length).toBe(0); expect(observed).toHaveBeenCalledOnce(); expect(reopened.getSnapshot()).toMatchObject({phase: "started", startRecovery: {phase: "done"}});
  expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(queue); expect(queue.inputs.map(input => input.inputId)).toEqual([f.tailId]);
});
it.each(["committed", "unconfirmed"] as const)("retains an original Started intent when cancellation status is contradictory: %s", async type => {
  const f = await fixture(); f.seedRelated(); const reopened = f.reopen(), observed = vi.fn();
  if (type === "unconfirmed") f.runtime.queue.failNextCancel("unavailable");
  else {
    const status: AgentWritingQueueCancelStatusResponse = {schemaVersion: 1, intent: f.cancelIntent, status: {type: "committed",
      proof: {schemaVersion: 1, operationId: f.cancelIntent.requestId, conversationId: f.scope.conversationId, family: "agent.queue.preview.cancel",
        epoch: crypto.randomUUID(), sqlFence: "12", replayed: false, result: null},
      queue: {schemaVersion: 1, ...f.scope, revision: f.cancelIntent.expectedRevision + 1, preparingInputId: null, paused: true,
        inputs: [{inputId: f.intent.inputId, text: "original request"}, {inputId: f.tailId, text: "tail preserved"}]}}};
    f.cancelStatus.mockResolvedValue(status);
  }
  await reopened.checkQueuedStart(observed);
  expect(reopened.getSnapshot()).toMatchObject({phase: "started", startRecovery: {phase: "localError", intent: f.intent,
    result: {alreadyStarted: true}, cause: {code: "QUEUE_START_CANCEL_UNCONFIRMED"}}});
  expect(f.storage.getItem(f.startKey)).not.toBeNull(); expect(f.storage.getItem(f.cancelKey)).not.toBeNull();
  expect(f.storage.getItem(f.prepareKey)).toBeNull(); expect(observed).toHaveBeenCalledOnce(); f.assertNoMutations();
});
it("a proved original start leaves a different R2 preparation and cancellation locator byte-for-byte intact", async () => {
  const f = await fixture(), newer = crypto.randomUUID(); f.seedRelated(newer);
  const prepared = f.storage.getItem(f.prepareKey), canceled = f.storage.getItem(f.cancelKey), reopened = f.reopen();
  await reopened.checkQueuedStart();
  expect(reopened.getSnapshot().startRecovery?.phase).toBe("done"); expect(f.storage.getItem(f.startKey)).toBeNull();
  expect(f.storage.getItem(f.prepareKey)).toBe(prepared); expect(f.storage.getItem(f.cancelKey)).toBe(canceled);
  expect(f.cancelStatus).not.toHaveBeenCalled(); expect(f.prepareStatus).not.toHaveBeenCalled(); f.assertNoMutations();
});
it("cleanup flush failure retries only local removal and reports the original admission once", async () => {
  const f = await fixture(); f.seedRelated(); const reopened = f.reopen(), observed = vi.fn(); let failing = true;
  persistence(f.storage, async () => {if (failing && !f.storage.getItem(f.startKey)) throw new Error("removal not durable");});
  await reopened.checkQueuedStart(observed);
  expect(reopened.getSnapshot()).toMatchObject({phase: "started", startRecovery: {phase: "localError", result: {alreadyStarted: true}}});
  expect(observed).toHaveBeenCalledOnce(); expect(f.storage.length).toBe(0); const statusCalls = f.status.mock.calls.length, cancelCalls = f.cancelStatus.mock.calls.length;
  failing = false; await reopened.retryQueuedStart(false, observed);
  expect(reopened.getSnapshot().startRecovery?.phase).toBe("done"); expect(observed).toHaveBeenCalledOnce();
  expect(f.status).toHaveBeenCalledTimes(statusCalls); expect(f.cancelStatus).toHaveBeenCalledTimes(cancelCalls); f.assertNoMutations();
});
it("the actual registry retains an in-flight queued start after its view releases and delivers admission to the original scope", async () => {
  const f = await fixture(false), key = `queued-detach:${crypto.randomUUID()}`; f.owner.dispose();
  const acquire = () => acquireWritingSubmissionController(key, f.runtime.gateways.submission, f.runtime.gateways.turns, f.runtime.queue.preview,
    {scope: f.scope, storage: f.storage});
  const owner = acquire(), observed = vi.fn(); cleanups.push(() => releaseWritingSubmissionController(key, owner));
  await owner.recoverQueued(f.scope);
  let release!: () => void; const pending = new Promise<void>(resolve => {release = resolve;});
  f.start.mockImplementationOnce(async intent => {await pending; return f.startOriginal(intent);});
  const work = owner.confirm("deepSeekNonThinking", observed);
  await vi.waitFor(() => expect(f.start).toHaveBeenCalledOnce());
  releaseWritingSubmissionController(key, owner); const reattached = acquire(); expect(reattached).toBe(owner);
  releaseWritingSubmissionController(key, reattached); release(); await work;
  expect(observed).toHaveBeenCalledExactlyOnceWith({scope: {...f.scope, requestId: f.preview.requestId, contextSnapshotId: f.preview.contextSnapshotId}, turnId: f.preview.turnId});
  expect(f.start).toHaveBeenCalledExactlyOnceWith(f.intent); expect(f.storage.length).toBe(0);
  const history = await f.runtime.history.gateway.readHistory({conversationId: f.scope.conversationId, afterSeq: 0, limit: 200});
  expect(history.events.filter(event => event.payload.type === "turnStarted")).toHaveLength(1);
});
it("the registry retains a Started owner with failed local cleanup and reattachment retries only that cleanup", async () => {
  const f = await fixture(), key = `queued-cleanup:${crypto.randomUUID()}`; f.owner.dispose();
  const acquire = () => acquireWritingSubmissionController(key, f.runtime.gateways.submission, f.runtime.gateways.turns, f.runtime.queue.preview,
    {scope: f.scope, storage: f.storage});
  const owner = acquire(), observed = vi.fn(); cleanups.push(() => releaseWritingSubmissionController(key, owner));
  let failing = true; persistence(f.storage, async () => {if (failing && !f.storage.getItem(f.startKey)) throw new Error("cleanup unavailable");});
  await owner.checkQueuedStart(observed);
  expect(owner.getSnapshot()).toMatchObject({phase: "started", startRecovery: {phase: "localError"}});
  releaseWritingSubmissionController(key, owner); const reattached = acquire(); expect(reattached).toBe(owner);
  const reads = f.status.mock.calls.length; failing = false; await reattached.retryQueuedStart(false, observed);
  expect(reattached.getSnapshot().startRecovery?.phase).toBe("done"); expect(f.status).toHaveBeenCalledTimes(reads);
  expect(observed).toHaveBeenCalledOnce(); f.assertNoMutations(); expect(f.storage.length).toBe(0);
});
it.each(["start", "status"] as const)("a disposed controller ignores a late %s result and cannot clear a newer R2 locator", async stage => {
  const f = await fixture(stage === "status"), owner = stage === "status" ? f.reopen() : f.owner, observed = vi.fn();
  let release!: () => void, reached = false; const pending = new Promise<void>(resolve => {release = resolve;});
  if (stage === "start") f.start.mockImplementationOnce(async intent => {const result = await f.startOriginal(intent); reached = true; await pending; return result;});
  else f.status.mockImplementationOnce(async intent => {const result = await f.statusOriginal(intent); reached = true; await pending; return result;});
  const work = stage === "start" ? owner.confirm("deepSeekNonThinking", observed) : owner.checkQueuedStart(observed);
  await vi.waitFor(() => expect(reached).toBe(true)); owner.dispose();
  const newer = JSON.parse(f.storage.getItem(f.startKey)!); newer.intent.start.requestId = crypto.randomUUID(); newer.turnId = crypto.randomUUID();
  newer.result = null; const retained = JSON.stringify(newer); f.storage.setItem(f.startKey, retained);
  release(); await work;
  expect(f.storage.getItem(f.startKey)).toBe(retained); expect(observed).not.toHaveBeenCalled();
  expect(f.start).toHaveBeenCalledOnce(); expect(f.prepare).not.toHaveBeenCalled(); expect(f.cancel).not.toHaveBeenCalled();
});

it("waits for durable known result before registering token-owned admission and retries only local finish", async () => {
  const f = await fixture(false, true); let failing = true;
  persistence(f.storage, async () => {if (failing && JSON.parse(f.storage.getItem(f.startKey) ?? "null")?.result) throw new Error("known result not durable");});
  await f.owner.confirm("deepSeekNonThinking", f.admitted);
  expect(f.owner.getSnapshot()).toMatchObject({phase: "started", startRecovery: {phase: "localError", result: {requestId: f.intent.start.requestId}, draftAcknowledgment: f.token}});
  expect(f.admitted).not.toHaveBeenCalled(); expect(f.drafts.read(f.draftKey)).toBe("original request"); expect(f.storage.getItem(f.prepareKey)).not.toBeNull();
  f.assertNoMutations(); expect(f.status).not.toHaveBeenCalled();
  failing = false; await f.owner.retryQueuedStart(false, f.admitted);
  expect(f.owner.getSnapshot().startRecovery?.phase).toBe("done"); expect(f.admitted).toHaveBeenCalledOnce(); expect(f.drafts.read(f.draftKey)).toBe("");
  expect(f.status).not.toHaveBeenCalled(); f.assertNoMutations();
});
it("keeps original workspace admission registered through token draft-flush failure without repeating start or clearing newer input", async () => {
  const f = await fixture(false, true); let calls = 0, failing = true;
  persistence(f.draftStorage, async () => {if (++calls === 2 && failing) throw new Error("draft deletion not durable");});
  await f.owner.confirm("deepSeekNonThinking", f.admitted);
  expect(f.owner.getSnapshot()).toMatchObject({phase: "started", startRecovery: {phase: "localError", result: {requestId: f.intent.start.requestId}}});
  expect(f.admitted).toHaveBeenCalledOnce(); expect(JSON.parse(f.storage.getItem(f.startKey)!).result.requestId).toBe(f.intent.start.requestId);
  expect(f.storage.getItem(f.prepareKey)).not.toBeNull(); f.assertNoMutations();
  f.drafts.update(f.draftKey, "original request"); failing = false; await f.owner.retryQueuedStart(false, f.admitted);
  expect(f.drafts.read(f.draftKey)).toBe("original request"); expect(f.owner.getSnapshot().startRecovery?.phase).toBe("done");
  expect(f.admitted).toHaveBeenCalledOnce(); expect(f.status).not.toHaveBeenCalled(); f.assertNoMutations();
});
it("defers token-owned statusStarted delivery until the known-result save recovers locally", async () => {
  const f = await fixture(true, true), reopened = f.reopen(), observed = vi.fn(); let failing = true;
  persistence(f.storage, async () => {if (failing && JSON.parse(f.storage.getItem(f.startKey) ?? "null")?.result) throw new Error("status result not durable");});
  await reopened.checkQueuedStart(observed);
  expect(reopened.getSnapshot()).toMatchObject({phase: "started", startRecovery: {phase: "localError", result: {alreadyStarted: true}, draftAcknowledgment: f.token}});
  expect(observed).not.toHaveBeenCalled(); expect(f.drafts.read(f.draftKey)).toBe("original request"); expect(f.status).toHaveBeenCalledOnce();
  failing = false; await reopened.retryQueuedStart(false, observed);
  expect(reopened.getSnapshot().startRecovery?.phase).toBe("done"); expect(f.drafts.read(f.draftKey)).toBe("");
  expect(observed).toHaveBeenCalledOnce(); expect(f.status).toHaveBeenCalledOnce(); f.assertNoMutations();
});
