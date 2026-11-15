import {afterEach, expect, it, vi} from "vitest";
import {decodeAgentWritingQueuePreviewRequest} from "@research-assistant/core/agent";
import {WritingSubmissionController} from "./writingSubmissionController";
import {WritingDiffDemoStorage} from "../../../app/previews/agent/WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "../../../app/previews/agent/WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "../../../app/previews/agent/writingDiffDemoRuntime";
import {registerStoragePersistence} from "../../runtime/storagePersistence";
import {selectAppStorage} from "../../runtime/appStorage";
import {writingQueuePreparationMessage} from "./writingQueuePreparationMessage";
import {holdRejectedQueuePreparation} from "./holdRejectedQueuePreparation";
import {writingQueuedStartQueue} from "./writingQueuedStartQueue";
class MemoryStorage implements Storage {
  readonly values = new Map<string, string>(); get length() {return this.values.size;}
  key(index: number) {return [...this.values.keys()][index] ?? null;}
  getItem(key: string) {return this.values.get(key) ?? null;} setItem(key: string, value: string) {this.values.set(key, value);}
  removeItem(key: string) {this.values.delete(key);} clear() {this.values.clear();}
}
const cleanups: (() => void)[] = [];
afterEach(() => {for (const cleanup of cleanups.splice(0)) cleanup(); vi.restoreAllMocks();});
async function fixture(tail = false, isAdmissionDisabled?: () => boolean) {
  const storage = new MemoryStorage(); selectAppStorage(storage);
  const scope = {manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID()}, inputId = crypto.randomUUID();
  const files = new WritingDiffDemoStorage(scope.manuscriptId, []), agent = new WritingDiffDemoAgentStorage(files, undefined);
  const runtime = createWritingDiffDemoRuntime(files, agent, scope.manuscriptId, scope.conversationId);
  let added = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: 0, edit: {type: "add", inputId, text: "original input", userInput: []}});
  if (tail) added = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: added.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "preserved tail", userInput: []}});
  const request = {conversationId: scope.conversationId, requestId: crypto.randomUUID(), text: "original input", context: {manuscriptId: scope.manuscriptId, files: []},
    resources: [{kind: "manuscript" as const, resourceId: scope.manuscriptId, parts: [], groupAccess: "readOnly" as const, newMemberAccess: "readOnly" as const}]};
  const controllers: WritingSubmissionController[] = [];
  const reopen = () => {const controller = new WritingSubmissionController(runtime.gateways.submission, runtime.gateways.turns, runtime.queue.preview, {scope, storage}, isAdmissionDisabled);
    controller.configureQueuedStart(writingQueuedStartQueue(() => runtime.gateways.queue.registry.acquire(scope), runtime.queue.gateway.read)); controllers.push(controller); return controller;};
  const prepareOriginal = runtime.queue.preview.prepare, startOriginal = runtime.queue.preview.start;
  const controller = reopen(), prepare = vi.spyOn(runtime.queue.preview, "prepare"), status = vi.spyOn(runtime.queue.preview, "prepareStatus"), start = vi.spyOn(runtime.queue.preview, "start");
  cleanups.push(() => {controllers.forEach(value => value.dispose()); runtime.dispose();});
  const begin = () => controller.prepareQueued(request, inputId, added.revision);
  const workspaceReady = async (owner = controller) => {const preview = owner.getSnapshot().preview!;
    await runtime.gateways.workspace.prepare({scope: {...scope, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId}, buffers: []});
    await owner.observeWorkspacePreparation(preview.requestId, true);};
  const retire = (owner = controller) => owner.retireQueuedPreparation((intent, eligible) => holdRejectedQueuePreparation(runtime.gateways.queue.registry.acquire(scope), intent, eligible));
  return {scope, inputId, request, storage, runtime, controller, reopen, begin, prepare, status, start, prepareOriginal, startOriginal, workspaceReady, retire};
}
function persistence(storage: Storage, flush: () => Promise<void>, retry = async () => {}, error: () => Error | undefined = () => undefined) {
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, get error() {return error();}, resetting: false, flush, retry, reset: vi.fn(), subscribe: vi.fn(() => () => {}), close: vi.fn()});
}
it("reliably flushes the full input before dispatch and a failed flush cannot prepare a model", async () => {
  const f = await fixture(); let failing = true; const retry = vi.fn(async () => {failing = false;});
  persistence(f.storage, async () => {if (failing) throw new Error("quota");}, retry, () => failing ? new Error("quota") : undefined);
  await f.begin(); expect(f.prepare).not.toHaveBeenCalled(); expect(f.status).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
  const intent = f.controller.getSnapshot().preparation!.intent!;
  expect(JSON.parse([...f.storage.values.values()][0]!).intent).toEqual(intent); expect(intent.submission.resources).toEqual(f.request.resources);
  await f.controller.retryQueuedPreparation(); expect(retry).toHaveBeenCalledOnce(); expect(f.prepare).toHaveBeenCalledExactlyOnceWith(intent);
  expect(f.controller.getSnapshot().phase).toBe("preview"); expect(f.storage.length).toBe(1);
  await f.workspaceReady(); expect(f.storage.length).toBe(0);
});
it("reopens a lost prepare ACK without rereading sources and installs only current queue Q", async () => {
  const f = await fixture(); f.runtime.queue.failNextPrepare("afterCommit"); await f.begin();
  const intent = f.controller.getSnapshot().preparation!.intent!;
  const frozen = await f.runtime.queue.preview.prepareStatus(intent); expect(frozen.status.type).toBe("committed"); f.status.mockClear();
  const current = await f.runtime.queue.gateway.read(f.scope), latest = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: current.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "later tail"}});
  f.controller.dispose(); const reopened = f.reopen();
  expect(reopened.getSnapshot()).toMatchObject({phase: "error", error: "prepare", request: null, preparation: {intent}});
  expect(f.prepare).toHaveBeenCalledOnce(); expect(f.status).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
  await reopened.restoreQueuedPreparation(); expect(f.status).toHaveBeenCalledExactlyOnceWith(intent); expect(f.prepare).toHaveBeenCalledOnce();
  expect(reopened.getSnapshot()).toMatchObject({phase: "preview", request: null, queue: latest});
  const restored = reopened.getSnapshot().preview!;
  await f.runtime.gateways.workspace.prepare({scope: {...f.scope, requestId: restored.requestId, contextSnapshotId: restored.contextSnapshotId}, buffers: []});
  await reopened.observeWorkspacePreparation(restored.requestId, true);
  await reopened.confirm("deepSeekNonThinking"); expect(f.start.mock.calls[0]![0].expectedRevision).toBe(latest.revision);
});
it("stops a queued prepare with a lost reply by checking and pausing the original input", async () => {
  const f = await fixture(true), cancel = vi.spyOn(f.runtime.queue.preview, "cancel");
  f.runtime.queue.failNextPrepare("afterCommit"); await f.begin();
  expect(await f.controller.cancelPendingSend(f.request.requestId)).toBe(true);
  const queue = await f.runtime.queue.gateway.read(f.scope);
  expect(queue).toMatchObject({paused:true,preparingInputId:null});
  expect(queue.inputs.some(item => item.inputId === f.inputId)).toBe(true);
  expect(f.status).toHaveBeenCalledOnce(); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.start).not.toHaveBeenCalled();
  expect(cancel).toHaveBeenCalledOnce(); expect(f.controller.isCancelPending()).toBe(false);
});
it("reopens an in-flight stopped queue prepare and pauses the original input without replaying prepare", async () => {
  const f = await fixture(true), cancel = vi.spyOn(f.runtime.queue.preview, "cancel");
  let release!: () => void; const gate = new Promise<void>(resolve => {release = resolve;});
  f.prepare.mockImplementationOnce(async intent => {await f.prepareOriginal(intent); await gate; throw new Error("lost reply");});
  const work = f.begin(); await vi.waitFor(() => expect(f.prepare).toHaveBeenCalledOnce());
  expect(await f.controller.cancelPendingSend(f.request.requestId)).toBe(true); f.controller.dispose();
  const reopened = f.reopen(); release(); await work;
  await vi.waitFor(() => expect(reopened.isCancelPending()).toBe(false));
  const queue = await f.runtime.queue.gateway.read(f.scope);
  expect(queue).toMatchObject({paused:true,preparingInputId:null});
  expect(queue.inputs.some(item => item.inputId === f.inputId)).toBe(true);
  expect(f.status).toHaveBeenCalledOnce(); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.start).not.toHaveBeenCalled();
  expect(cancel).toHaveBeenCalledOnce();
});
it("surfaces an unconfirmed stopped queue prepare and retries only its original status", async () => {
  const f = await fixture(); f.runtime.queue.failNextPrepare("beforeCommit"); await f.begin();
  expect(await f.controller.cancelPendingSend(f.request.requestId)).toBe(true);
  expect(f.controller.getSnapshot()).toMatchObject({cancelSendRequested:true,sendStopNeedsReview:true,preparation:{phase:"notFound"}});
  expect(f.prepare).toHaveBeenCalledOnce(); expect(f.status).toHaveBeenCalledOnce();
  expect(await f.controller.retrySendStopIntent(f.scope)).toBe(true);
  expect(f.controller.getSnapshot().sendStopNeedsReview).toBe(true);
  expect(f.status).toHaveBeenCalledTimes(2); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.start).not.toHaveBeenCalled();
});
it("checks notFound readonly and explicit retry replays original full intent after the caller changes", async () => {
  const f = await fixture(); f.runtime.queue.failNextPrepare("beforeCommit"); await f.begin(); const intent = f.controller.getSnapshot().preparation!.intent!;
  f.request.text = "later draft"; f.request.resources[0]!.newMemberAccess = "readOnly";
  f.controller.dispose(); const reopened = f.reopen(); await reopened.restoreQueuedPreparation();
  expect(reopened.getSnapshot().preparation?.phase).toBe("notFound"); expect(f.prepare).toHaveBeenCalledOnce();
  await reopened.retryQueuedPreparation(); expect(f.prepare.mock.calls.map(([value]) => value)).toEqual([intent, intent]); expect(f.start).not.toHaveBeenCalled();
});
it("retains Unconfirmed and its safe diagnostic, never changes R or Q on explicit replay", async () => {
  const f = await fixture(); f.runtime.queue.failNextPrepare("unavailable"); await f.begin(); const intent = f.controller.getSnapshot().preparation!.intent!;
  await f.controller.restoreQueuedPreparation(); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.controller.getSnapshot().preparation?.phase).toBe("unconfirmed");
  const message = writingQueuePreparationMessage(f.controller.getSnapshot().preparation);
  for (const identity of [intent.submission.conversationId, intent.inputId, intent.submission.requestId, "mock_owner_unavailable"]) expect(message).toContain(identity);
  await f.controller.retryQueuedPreparation(); expect(f.prepare.mock.calls.map(([value]) => value)).toEqual([intent, intent]); expect(f.start).not.toHaveBeenCalled();
});
it("known committed cleanup failure only retries local cleanup and then reads the current claim", async () => {
  const f = await fixture(); let fail = true;
  persistence(f.storage, async () => {if (fail && ![...f.storage.values.keys()].some(key => key.includes("queue-prepare"))) throw new Error("cleanup");});
  await f.begin(); await f.workspaceReady(); expect(f.controller.getSnapshot().preparation).toMatchObject({knownCommitted: true, phase: "localError"});
  expect(f.prepare).toHaveBeenCalledOnce(); fail = false; await f.controller.retryQueuedPreparation();
  expect(f.prepare).toHaveBeenCalledOnce(); expect(f.status).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled(); expect(f.controller.getSnapshot().phase).toBe("preview");
});
it("a committed claim read error can retry only the read, then a successful absent pointer retires the UI claim", async () => {
  const f = await fixture(), recover = vi.spyOn(f.runtime.queue.preview, "recover").mockRejectedValueOnce(new Error("offline"));
  await f.begin(); expect(f.controller.getSnapshot()).toMatchObject({error: "prepare", preparation: {knownCommitted: true, phase: "ready"}});
  recover.mockResolvedValueOnce(null); await f.controller.restoreQueuedPreparation();
  expect(f.controller.getSnapshot().phase).toBe("idle"); expect(f.controller.hasQueuedPreparation()).toBe(false);
  expect(f.prepare).toHaveBeenCalledOnce(); expect(f.status).not.toHaveBeenCalled(); expect(f.start).not.toHaveBeenCalled();
});
it("a committed historical prepare never authorizes the later R occupying the same I", async () => {
  const f = await fixture(); f.runtime.queue.failNextPrepare("afterCommit"); await f.begin();
  const original = f.controller.getSnapshot().preparation!.intent!, committed = await f.runtime.queue.preview.prepareStatus(original);
  if (committed.status.type !== "committed") throw new Error("fixture missing receipt");
  await f.runtime.queue.preview.cancel({...f.scope, inputId: f.inputId, requestId: crypto.randomUUID(), previewRequestId: original.submission.requestId, expectedRevision: committed.status.result.queue.revision});
  const q = await f.runtime.queue.gateway.read(f.scope), resumed = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: q.revision, edit: {type: "resumeInput", inputId: f.inputId}});
  const newer = await f.runtime.queue.preview.prepare({inputId: f.inputId, expectedRevision: resumed.revision, submission: {...f.request, requestId: crypto.randomUUID()}});
  const count = f.prepare.mock.calls.length; await f.controller.restoreQueuedPreparation();
  expect(f.controller.getSnapshot().phase).toBe("idle"); expect(f.controller.hasQueuedPreparation()).toBe(false); expect(f.prepare).toHaveBeenCalledTimes(count); expect(f.start).not.toHaveBeenCalled();
  expect((await f.runtime.queue.preview.recover(f.scope))?.preview.requestId).toBe(newer.preview.requestId);
});
it("late ACK after disposal cannot remove a newer scope locator", async () => {
  const f = await fixture(); let finish!: () => void; const actual = f.prepareOriginal;
  f.prepare.mockImplementationOnce(async intent => {const result = await actual(intent); await new Promise<void>(resolve => {finish = resolve;}); return result;});
  const work = f.begin(); await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
  f.controller.dispose(); const locator = [...f.storage.values.keys()][0]!, newer = "foreign locator"; f.storage.setItem(locator, newer); finish(); await work;
  expect(f.storage.getItem(locator)).toBe(newer); expect(f.start).not.toHaveBeenCalled();
});
it("a late definitive failure after disposal cannot rewrite retained recovery evidence", async () => {
  const f = await fixture(); let reject!: (cause: unknown) => void;
  f.prepare.mockImplementationOnce(() => new Promise((_, fail) => {reject = fail;})); const work = f.begin();
  await vi.waitFor(() => expect(reject).toBeTypeOf("function")); const before = [...f.storage.values.values()][0]; f.controller.dispose();
  reject({code: "storage", reason: {...f.scope, inputId: f.inputId, previewRequestId: f.request.requestId, operationId: f.request.requestId,
    code: "DATA_SQL_DOMAIN", reason: "conversation changed since it was read"}}); await work;
  expect([...f.storage.values.values()][0]).toBe(before);
});
it("disposal while reliable save is pending cannot dispatch prepare", async () => {
  const f = await fixture(); let finish!: () => void;
  persistence(f.storage, () => new Promise<void>(resolve => {finish = resolve;})); const work = f.begin();
  await vi.waitFor(() => expect(finish).toBeTypeOf("function")); f.controller.dispose(); finish(); await work; expect(f.prepare).not.toHaveBeenCalled();
});
it("reads the actual Started fact after ACK loss even when pointer is removed and Q advances", async () => {
  const f = await fixture(); await f.begin(); const preview = f.controller.getSnapshot().preview!;
  await f.runtime.gateways.workspace.prepare({scope: {...f.scope, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId}, buffers: []});
  await f.controller.observeWorkspacePreparation(preview.requestId, true);
  const current = await f.runtime.queue.gateway.read(f.scope), appended = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: current.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "tail"}});
  const actual = f.startOriginal; f.start.mockImplementationOnce(async value => {await actual(value); throw new Error("lost start ACK");});
  await f.controller.confirm("deepSeekNonThinking"); expect(f.controller.getSnapshot().error).toBe("start"); expect(await f.runtime.queue.preview.recover(f.scope)).toBeNull();
  const original = f.start.mock.calls[0]![0], status = vi.spyOn(f.runtime.queue.preview, "startStatus");
  const admitted = await f.runtime.queue.gateway.read(f.scope); await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: admitted.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "later tail"}});
  await f.controller.confirm("deepSeekNonThinking"); expect(status).toHaveBeenCalledExactlyOnceWith(original);
  expect(f.start).toHaveBeenCalledOnce(); expect(original.expectedRevision).toBe(appended.revision);
  expect(f.controller.getSnapshot().phase).toBe("started");
  const history = await f.runtime.gateways.history!.readHistory({conversationId: f.scope.conversationId, afterSeq: 0, limit: 100});
  expect(history.events.filter(event => event.payload.type === "turnStarted")).toHaveLength(1);
});
it("a first native queued CAS rejection retries the same R with a newly read current Q", async () => {
  const f = await fixture(); await f.begin(); await f.workspaceReady();
  f.start.mockImplementationOnce(async () => {
    const current = await f.runtime.queue.gateway.read(f.scope); await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: current.revision,
      edit: {type: "add", inputId: crypto.randomUUID(), text: "concurrent tail"}});
    throw {code: "preparation", reason: "conflict"};
  });
  await f.controller.confirm("deepSeekNonThinking"); expect(f.controller.getSnapshot().error).toBe("startRejected");
  await f.controller.confirm("deepSeekNonThinking"); expect(f.start.mock.calls[1]![0].expectedRevision).toBe(f.start.mock.calls[0]![0].expectedRevision + 1);
  expect(f.start.mock.calls[1]![0].start).toEqual(f.start.mock.calls[0]![0].start); expect(f.controller.getSnapshot().phase).toBe("started");
  expect((await f.runtime.queue.gateway.read(f.scope)).inputs.map(input => input.text)).toEqual(["concurrent tail"]);
});
it("a native CAS-shaped rejection after an unknown ACK still cannot replace the actual start Q", async () => {
  const f = await fixture(); await f.begin(); await f.workspaceReady();
  f.runtime.queue.failNextStart("afterAdmission");
  await f.controller.confirm("deepSeekNonThinking"); expect(f.controller.getSnapshot().error).toBe("start");
  const current = await f.runtime.queue.gateway.read(f.scope); await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: current.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "later tail"}});
  f.start.mockRejectedValueOnce({code: "preparation", reason: "conflict"}); await f.controller.confirm("deepSeekNonThinking");
  expect(f.controller.getSnapshot().error).toBe("start"); await f.controller.confirm("deepSeekNonThinking");
  expect(f.start.mock.calls[1]).toEqual(f.start.mock.calls[0]); expect(f.start.mock.calls[2]).toEqual(f.start.mock.calls[0]);
  expect(f.controller.getSnapshot().phase).toBe("started");
});
it("a known stale-Q rejection survives reopen and pauses the retained input only after NotFound", async () => {
  const f = await fixture(); const current = await f.runtime.queue.gateway.read(f.scope), latest = await f.runtime.queue.gateway.edit({...f.scope,
    requestId: crypto.randomUUID(), expectedRevision: current.revision, edit: {type: "add", inputId: crypto.randomUUID(), text: "tail"}});
  await f.begin(); expect(f.controller.getSnapshot().preparation?.definitiveRejection).toBe(true);
  f.controller.dispose(); const reopened = f.reopen(); expect(reopened.getSnapshot().preparation?.definitiveRejection).toBe(true);
  expect(writingQueuePreparationMessage(reopened.getSnapshot().preparation)).toContain("DATA_SQL_DOMAIN");
  await reopened.restoreQueuedPreparation(); expect(f.prepare).toHaveBeenCalledOnce(); expect(reopened.getSnapshot().preparation?.phase).toBe("notFound");
  await f.retire(reopened); expect(reopened.getSnapshot().phase).toBe("idle"); expect(reopened.hasQueuedPreparation()).toBe(false);
  expect(f.storage.length).toBe(0); expect(await f.runtime.queue.gateway.read(f.scope)).toMatchObject({...latest, revision: latest.revision + 1, inputs: [{inputId: f.inputId, text: "original input", userInput: [], paused: true}, latest.inputs[1]]}); expect(f.start).not.toHaveBeenCalled();
});
it.each(["context", "model"])("accepts only exact known prewrite %s rejection evidence without repeating prepare", async kind => {
  const f = await fixture(); const code = kind === "context" ? "DATA_SQL_OPERATION_CONFLICT" : "DATA_SQL_UNAVAILABLE";
  f.prepare.mockRejectedValueOnce({code: "storage", reason: {...f.scope, inputId: f.inputId, previewRequestId: f.request.requestId, operationId: f.request.requestId,
    code, reason: {operationId: f.request.requestId, epoch: null, sqlFence: null, sqliteCode: null, sqliteExtendedCode: null,
      phase: kind === "context" ? "queue-prepare-context" : "queue-prepare-model-selection", safeCause: kind === "context" ? "writing_context_changed" : "model_credentials_unavailable"}}});
  await f.begin(); f.controller.dispose(); const reopened = f.reopen(); expect(reopened.getSnapshot().preparation?.definitiveRejection).toBe(true);
  expect(writingQueuePreparationMessage(reopened.getSnapshot().preparation)).toContain(code);
  await f.retire(reopened); expect(f.status).toHaveBeenCalledOnce(); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.start).not.toHaveBeenCalled(); expect(f.storage.length).toBe(0);
});
it.each([
  ["DATA_SQL_OPERATION_CONFLICT", "queue-prepare-input", "writing_input_changed"],
  ["DATA_SQL_OPERATION_CONFLICT", "queue-prepare-instructions", "writing_instructions_unauthorized"],
  ["DATA_SQL_INVALID", "queue-prepare-context", "writing_context_invalid"],
  ["DATA_SQL_INVALID", "queue-prepare-context", "writing_context_budget_exceeded"],
  ["DATA_SQL_INVALID", "queue-prepare-instructions", "writing_instructions_scope_invalid"],
  ["DATA_SQL_INVALID", "queue-prepare-instructions", "writing_instructions_budget_exceeded"],
  ["DATA_SQL_INVALID", "queue-prepare-instructions", "writing_instructions_encoding_invalid"],
  ["DATA_SQL_INVALID", "queue-prepare-input", "writing_input_invalid"],
])("retires only the proved rollback %s/%s/%s after pausing the original input", async (code, phase, safeCause) => {
  const f = await fixture(); f.prepare.mockRejectedValueOnce({code: "storage", reason: {...f.scope, inputId: f.inputId, previewRequestId: f.request.requestId, operationId: f.request.requestId,
    code, reason: {operationId: f.request.requestId, epoch: null, sqlFence: null, sqliteCode: null, sqliteExtendedCode: null, phase, safeCause}}});
  await f.begin(); f.controller.dispose(); const restored = f.reopen(); expect(restored.getSnapshot().preparation?.definitiveRejection).toBe(true);
  await f.retire(restored); expect(restored.getSnapshot().phase).toBe("idle"); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.start).not.toHaveBeenCalled();
  expect((await f.runtime.queue.gateway.read(f.scope)).inputs[0]).toMatchObject({inputId: f.inputId, paused: true});
});
it.each(["phase", "cause", "outerId", "innerId", "code"])("a mismatched %s cannot confer materialization rollback authority", async mismatch => {
  const f = await fixture(); f.prepare.mockRejectedValueOnce({code: "storage", reason: {...f.scope, inputId: mismatch === "outerId" ? crypto.randomUUID() : f.inputId,
    previewRequestId: f.request.requestId, operationId: f.request.requestId, code: mismatch === "code" ? "DATA_SQL_CORRUPT" : "DATA_SQL_INVALID",
    reason: {operationId: mismatch === "innerId" ? crypto.randomUUID() : f.request.requestId, epoch: null, sqlFence: null, sqliteCode: null, sqliteExtendedCode: null,
      phase: mismatch === "phase" ? "queue-prepare-input" : "queue-prepare-context", safeCause: mismatch === "cause" ? "writing_input_invalid" : "writing_context_invalid"}}});
  await f.begin(); await f.controller.restoreQueuedPreparation(); await f.retire(); expect(f.storage.length).toBe(1);
  expect(f.controller.getSnapshot().preparation?.definitiveRejection).toBe(false); expect((await f.runtime.queue.gateway.read(f.scope)).inputs[0]?.paused).not.toBe(true);
});
it("Unknown plus NotFound or a foreign purported rejection never authorizes dropping the original intent", async () => {
  const f = await fixture(); f.runtime.queue.failNextPrepare("beforeCommit"); await f.begin(); await f.controller.restoreQueuedPreparation();
  await f.retire(); expect(f.storage.length).toBe(1); expect(f.controller.getSnapshot().preparation?.phase).toBe("notFound");
  expect(f.controller.hasQueuedPreparation()).toBe(true); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.start).not.toHaveBeenCalled();
});
it("return-to-queue cleanup failure only retries local removal and leaves the next preparation independent", async () => {
  const f = await fixture(); const q = await f.runtime.queue.gateway.read(f.scope); await f.runtime.queue.gateway.edit({...f.scope,
    requestId: crypto.randomUUID(), expectedRevision: q.revision, edit: {type: "add", inputId: crypto.randomUUID(), text: "tail"}});
  await f.begin(); let failing = true;
  persistence(f.storage, async () => {if (failing && !f.storage.length) throw new Error("cleanup");});
  await f.retire(); expect(f.controller.getSnapshot().preparation?.phase).toBe("localError");
  const checks = f.status.mock.calls.length; failing = false; await f.retire();
  expect(f.status).toHaveBeenCalledTimes(checks); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.controller.getSnapshot().phase).toBe("idle");
  const held = await f.runtime.queue.gateway.read(f.scope), resumed = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: held.revision, edit: {type: "resumeInput", inputId: f.inputId}});
  const next = {...f.request, requestId: crypto.randomUUID()}; await f.controller.prepareQueued(next, f.inputId, resumed.revision);
  expect(f.controller.getSnapshot()).toMatchObject({phase: "preview", preparation: {definitiveRejection: false}});
});
it.each(["foreign", "unknown"])("a %s failure cannot grant rejection retirement authority", async kind => {
  const f = await fixture(); f.prepare.mockRejectedValueOnce({code: "storage", reason: {...f.scope, inputId: kind === "foreign" ? crypto.randomUUID() : f.inputId,
    previewRequestId: f.request.requestId, operationId: f.request.requestId, code: kind === "foreign" ? "DATA_SQL_DOMAIN" : "DATA_SQL_UNAVAILABLE",
    reason: kind === "foreign" ? "conversation changed since it was read" : {operationId: f.request.requestId, phase: "worker", safeCause: "model_credentials_unavailable"}}});
  await f.begin(); await f.controller.restoreQueuedPreparation(); await f.retire();
  expect(f.controller.getSnapshot().preparation?.definitiveRejection).toBe(false); expect(f.storage.length).toBe(1); expect(f.prepare).toHaveBeenCalledOnce();
});
it.each([false, true])("cancel proof clears its original prepare locator after both reopen (prepare ACK lost=%s)", async lostAck => {
  const f = await fixture(true), cancel = vi.spyOn(f.runtime.queue.preview, "cancel"), cancelStatus = vi.spyOn(f.runtime.queue.preview, "cancelStatus");
  if (lostAck) f.runtime.queue.failNextPrepare("afterCommit"); await f.begin();
  if (lostAck) await f.controller.restoreQueuedPreparation();
  const preview = f.controller.getSnapshot().preview!; f.status.mockClear(); f.runtime.queue.failNextCancel("afterCommit");
  await f.controller.cancelQueued(crypto.randomUUID()); expect(f.storage.length).toBe(2);
  const intent = f.controller.getSnapshot().cancellation!.intent!, paused = await f.runtime.queue.gateway.read(f.scope);
  f.controller.dispose(); const reopened = f.reopen();
  expect(reopened.getSnapshot()).toMatchObject({phase: "error", error: "cancel", cancellation: {intent}});
  expect(cancelStatus).not.toHaveBeenCalled(); expect(f.status).not.toHaveBeenCalled();
  await reopened.checkQueuedCancellation();
  expect(cancelStatus).toHaveBeenCalledExactlyOnceWith(intent); expect(cancel).toHaveBeenCalledOnce(); expect(f.prepare).toHaveBeenCalledOnce(); expect(f.status).not.toHaveBeenCalled();
  expect(f.storage.length).toBe(0); expect(reopened.getSnapshot()).toMatchObject({phase: "idle"});
  expect(paused).toMatchObject({paused: true, preparingInputId: null, inputs: [{inputId: f.inputId}, {text: "preserved tail"}]});
  expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(paused);
  const resumed = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: paused.revision, edit: {type: "resumeInput", inputId: f.inputId}});
  const next = {...f.request, requestId: crypto.randomUUID()}; await reopened.prepareQueued(next, f.inputId, resumed.revision);
  expect(reopened.getSnapshot()).toMatchObject({phase: "preview", preview: {requestId: next.requestId}});
  expect(next.requestId).not.toBe(preview.requestId); expect(f.prepare).toHaveBeenCalledTimes(2); expect(f.start).not.toHaveBeenCalled();
});
it("an old cancellation proof cannot clear a newer same-scope prepare locator", async () => {
  const f = await fixture(true); await f.begin(); f.runtime.queue.failNextCancel("afterCommit"); await f.controller.cancelQueued(crypto.randomUUID());
  const prepKey = [...f.storage.values.keys()].find(key => key.includes("queue-prepare"))!, previous = JSON.parse(f.storage.getItem(prepKey)!);
  const paused = await f.runtime.queue.gateway.read(f.scope), resumed = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: paused.revision,
    edit: {type: "resumeInput", inputId: f.inputId}});
  const newer = decodeAgentWritingQueuePreviewRequest({inputId: f.inputId, expectedRevision: resumed.revision, submission: {...f.request, requestId: crypto.randomUUID()}});
  await f.runtime.queue.preview.prepare(newer);
  const bytes = JSON.stringify({...previous, intent: newer, knownCommitted: true}); f.storage.setItem(prepKey, bytes);
  f.controller.dispose(); const reopened = f.reopen(), prepareCalls = f.prepare.mock.calls.length, cancelStatus = vi.spyOn(f.runtime.queue.preview, "cancelStatus");
  await reopened.checkQueuedCancellation();
  expect(cancelStatus).toHaveBeenCalledOnce(); expect(f.storage.getItem(prepKey)).toBe(bytes); expect(f.storage.length).toBe(1);
  expect(f.prepare).toHaveBeenCalledTimes(prepareCalls); expect((await f.runtime.queue.preview.recover(f.scope))?.preview.requestId).toBe(newer.submission.requestId);
  await reopened.recoverQueued(f.scope); await reopened.restoreQueuedPreparation();
  expect(reopened.getSnapshot()).toMatchObject({phase: "preview", preview: {requestId: newer.submission.requestId}}); expect(f.storage.length).toBe(1);
});
it("post-cancel prepare cleanup failure retries only local cleanup even without a local prepare ACK marker", async () => {
  const f = await fixture(true); await f.begin(); f.runtime.queue.failNextCancel("afterCommit"); await f.controller.cancelQueued(crypto.randomUUID());
  const prepKey = [...f.storage.values.keys()].find(key => key.includes("queue-prepare"))!, pending = JSON.parse(f.storage.getItem(prepKey)!);
  // The durable pre-dispatch envelope may survive when the prepare ACK marker
  // did not: cancellation still proves exactly which original claim it released.
  f.storage.setItem(prepKey, JSON.stringify({...pending, knownCommitted: false}));
  f.controller.dispose(); const reopened = f.reopen(); let failing = true;
  persistence(f.storage, async () => {if (failing && f.storage.length === 0) throw new Error("cleanup");});
  const cancelStatus = vi.spyOn(f.runtime.queue.preview, "cancelStatus"), cancel = vi.spyOn(f.runtime.queue.preview, "cancel");
  f.status.mockClear(); f.prepare.mockClear(); await reopened.checkQueuedCancellation();
  expect(reopened.getSnapshot()).toMatchObject({phase: "error", error: "prepare", preparation: {phase: "localError", knownCommitted: false}});
  expect(cancelStatus).toHaveBeenCalledOnce(); expect(cancel).not.toHaveBeenCalled(); expect(f.prepare).not.toHaveBeenCalled(); expect(f.status).not.toHaveBeenCalled();
  failing = false; await reopened.retryQueuedPreparation();
  expect(reopened.getSnapshot().phase).toBe("idle"); expect(f.storage.length).toBe(0);
  expect(cancelStatus).toHaveBeenCalledOnce(); expect(cancel).not.toHaveBeenCalled(); expect(f.prepare).not.toHaveBeenCalled(); expect(f.status).not.toHaveBeenCalled();
  expect(await f.runtime.queue.gateway.read(f.scope)).toMatchObject({paused: true, preparingInputId: null, inputs: [{inputId: f.inputId}, {text: "preserved tail"}]});
});
it("retiring the canceled prepare cannot acknowledge its old draft when another request is later recovered", async () => {
  const f = await fixture(), acknowledged = vi.fn();
  const queue = await f.runtime.queue.gateway.read(f.scope); await f.controller.prepareQueued(f.request, f.inputId, queue.revision, acknowledged);
  await f.controller.cancelQueued(crypto.randomUUID()); expect(f.controller.getSnapshot().phase).toBe("idle");
  const paused = await f.runtime.queue.gateway.read(f.scope), resumed = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: paused.revision,
    edit: {type: "resumeInput", inputId: f.inputId}});
  const newer = await f.runtime.queue.preview.prepare({inputId: f.inputId, expectedRevision: resumed.revision, submission: {...f.request, requestId: crypto.randomUUID()}});
  await f.controller.recoverQueued(f.scope); await f.runtime.gateways.workspace.prepare({scope: {...f.scope, requestId: newer.preview.requestId, contextSnapshotId: newer.preview.contextSnapshotId}, buffers: []});
  await f.controller.observeWorkspacePreparation(newer.preview.requestId, true); await f.controller.confirm("deepSeekNonThinking");
  expect(f.controller.getSnapshot().phase).toBe("started"); expect(acknowledged).not.toHaveBeenCalled();
});

it("a warm queued preparation blocked before its first RPC can return through original hold and retirement", async () => {
  let disabled = false, release!: () => void;
  const f = await fixture(false, () => disabled), gate = new Promise<void>(resolve => {release = resolve;});
  persistence(f.storage, () => gate);
  const work = f.begin(); await vi.waitFor(() => expect(f.controller.getSnapshot().preparation?.intent).toBeDefined());
  disabled = true; release(); await work;
  expect(f.prepare).not.toHaveBeenCalled();
  expect(f.controller.getSnapshot().preparation).toMatchObject({phase: "localError", definitiveRejection: false, knownCommitted: false});
  expect(writingQueuePreparationMessage(f.controller.getSnapshot().preparation)).toContain("此步骤尚未派发");
  await f.retire();
  expect(f.controller.getSnapshot().preparation?.phase).toBe("retired");
  const queue = await f.runtime.queue.gateway.read(f.scope);
  expect(queue.inputs.find(input => input.inputId === f.inputId)?.paused).toBe(true);
  expect(f.prepare).not.toHaveBeenCalled();
});

it("retires only a warm blocked queued start locator before cancelling its original preview claim", async () => {
  let disabled = false, release!: () => void;
  const f = await fixture(false, () => disabled); await f.begin(); await f.workspaceReady();
  const claimed = await f.runtime.queue.gateway.read(f.scope), cancel = vi.spyOn(f.runtime.queue.preview, "cancel");
  expect(claimed.preparingInputId).toBe(f.inputId);
  const gate = new Promise<void>(resolve => {release = resolve;}); persistence(f.storage, () => gate);
  const work = f.controller.confirm("deepSeekNonThinking");
  await vi.waitFor(() => expect(f.controller.getSnapshot().startRecovery?.intent).not.toBeNull());
  disabled = true; release(); await work;
  expect(f.start).not.toHaveBeenCalled(); expect(cancel).not.toHaveBeenCalled();
  expect((await f.runtime.queue.gateway.read(f.scope)).preparingInputId).toBe(f.inputId);
  expect(await f.controller.cancelPendingSend(f.request.requestId)).toBe(true);
  expect(cancel).toHaveBeenCalledOnce(); expect(f.start).not.toHaveBeenCalled();
  expect(f.controller.isCancelPending()).toBe(false);
  expect(await f.runtime.queue.gateway.read(f.scope)).toMatchObject({preparingInputId: null, paused: true});
});

it.each(["prepare", "start"] as const)("checks queued %s after a synchronous sending subscriber closes admission", async step => {
  let disabled = false; const f = await fixture(false, () => disabled);
  const unsubscribe = f.controller.subscribe(() => {
    const state = f.controller.getSnapshot();
    if (step === "prepare" ? state.preparation?.phase === "sending" : state.startRecovery?.phase === "sending") disabled = true;
  });
  await f.begin();
  if (step === "prepare") expect(f.prepare).not.toHaveBeenCalled();
  else {await f.workspaceReady(); await f.controller.confirm("deepSeekNonThinking"); expect(f.start).not.toHaveBeenCalled();}
  expect(step === "prepare" ? f.controller.getSnapshot().preparation?.cause : f.controller.getSnapshot().startRecovery?.cause)
    .toMatchObject({code: "WRITING_DISPATCH_ADMISSION_CLOSED", step});
  unsubscribe();
});

it("checks the captured instance after confirm's original claim recovery await", async () => {
  let disabled = false; const f = await fixture(false, () => disabled); await f.begin(); await f.workspaceReady();
  const recover = f.runtime.queue.preview.recover;
  vi.spyOn(f.runtime.queue.preview, "recover").mockImplementationOnce(async scope => {const result = await recover(scope); disabled = true; return result;});
  await f.controller.confirm("deepSeekNonThinking");
  expect(f.start).not.toHaveBeenCalled(); expect(f.controller.getSnapshot().startRecovery?.cause).toMatchObject({code: "WRITING_DISPATCH_ADMISSION_CLOSED"});
  expect((await f.runtime.queue.gateway.read(f.scope)).preparingInputId).toBe(f.inputId);
});

it("requires fresh admission after a proved queued prepare rejection but keeps a later actual Unknown recoverable", async () => {
  let disabled = false; const f = await fixture(false, () => disabled);
  f.prepare.mockRejectedValueOnce({code: "storage", reason: {...f.scope, inputId: f.inputId, previewRequestId: f.request.requestId,
    operationId: f.request.requestId, code: "DATA_SQL_INVALID", reason: {operationId: f.request.requestId, epoch: null, sqlFence: null,
      sqliteCode: null, sqliteExtendedCode: null, phase: "queue-prepare-context", safeCause: "writing_context_invalid"}}});
  await f.begin(); expect(f.controller.getSnapshot().preparation?.definitiveRejection).toBe(true);
  disabled = true; await f.controller.retryQueuedPreparation(); expect(f.prepare).toHaveBeenCalledOnce();
  expect(f.controller.getSnapshot().preparation?.cause).toMatchObject({code: "WRITING_DISPATCH_ADMISSION_CLOSED", step: "prepare"});
  disabled = false; f.prepare.mockRejectedValueOnce(new Error("actual retry ACK lost")); await f.controller.retryQueuedPreparation();
  expect(f.prepare).toHaveBeenCalledTimes(2);
  disabled = true; f.prepare.mockRejectedValueOnce(new Error("retry still Unknown"));
  await f.controller.retryQueuedPreparation(); expect(f.prepare).toHaveBeenCalledTimes(3);
  expect(f.prepare.mock.calls[0]).toEqual(f.prepare.mock.calls[2]);
  f.controller.dispose(); const cold = f.reopen(); await cold.retryQueuedPreparation();
  expect(f.prepare).toHaveBeenCalledTimes(4); expect(f.prepare.mock.calls[0]).toEqual(f.prepare.mock.calls[3]);
});

it("shows exact Native source details in production queue preparation recovery", async () => {
  const f = await fixture();
  const requestId = f.request.requestId;
  const nativeId = "55555555-5555-4555-8555-555555555555";
  f.prepare.mockRejectedValue({code: "storage", reason: {...f.scope,
    inputId: f.inputId, previewRequestId: requestId, operationId: requestId,
    code: "DATA_SQL_OPERATION_CONFLICT", reason: {operationId: requestId,
      phase: "queue-prepare-context", safeCause: "writing_context_changed"}},
    writingSource: {...f.scope, inputId: f.inputId, operationId: requestId,
      source: {kind: "native", packageKey: "c".repeat(64), instanceId: nativeId,
        requestId: nativeId, rejection: "conflict", body: "PRIVATE_SOURCE"}},
  });
  await f.begin();
  const state = f.controller.getSnapshot().preparation;
  const message = writingQueuePreparationMessage(state);
  for (const token of ["writing_context_changed", `packageKey=${"c".repeat(64)}`, `nativeRequestId=${nativeId}`, "source.rejection=conflict"]) expect(message).toContain(token);
  expect(message).not.toContain("PRIVATE_SOURCE");
  expect(state?.definitiveRejection).toBe(true);
  expect(f.start).not.toHaveBeenCalled();
});
