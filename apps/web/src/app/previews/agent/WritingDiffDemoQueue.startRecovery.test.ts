import {afterEach, expect, it, vi} from "vitest";
import {decodeAgentWritingQueuedStartRequest, decodeAgentWritingQueuedStartStatusResponse, type AgentWritingQueuedStartRequest} from "@research-assistant/core/agent";
import * as CoreAgent from "@research-assistant/core/agent";
import {WritingDiffDemoStorage} from "./WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "./WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "./writingDiffDemoRuntime";
const cleanups: (() => void)[] = [];
afterEach(() => {cleanups.splice(0).forEach(cleanup => cleanup()); vi.restoreAllMocks();});
async function fixture({prepared = true, paused = false, text = "original queued request"} = {}) {
  const scope = {manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID()}, inputId = crypto.randomUUID(), tailId = crypto.randomUUID();
  const files = new WritingDiffDemoStorage(scope.manuscriptId, []), agent = new WritingDiffDemoAgentStorage(files, undefined), runtime = createWritingDiffDemoRuntime(files, agent, scope.manuscriptId, scope.conversationId);
  runtime.execution.historyOnly = true; cleanups.push(() => runtime.dispose());
  let queue = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: 0, edit: {type: "add", inputId, text}});
  queue = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: queue.revision, edit: {type: "add", inputId: tailId, text: "preserved tail"}});
  queue = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: queue.revision, edit: {type: "pauseInput", inputId: tailId}});
  const submission = {conversationId: scope.conversationId, requestId: crypto.randomUUID(), text, context: {manuscriptId: scope.manuscriptId, files: []}};
  const preparedRequest = {inputId, expectedRevision: queue.revision, submission};
  const preview = prepared ? (await runtime.queue.preview.prepare(preparedRequest)).preview : null;
  if (preview) await runtime.gateways.workspace.prepare({scope: {...scope, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId}, buffers: []});
  if (paused) runtime.queue.restore(runtime.queue.snapshot());
  const start = preview ? decodeAgentWritingQueuedStartRequest({inputId, expectedRevision: (await runtime.queue.gateway.read(scope)).revision,
    start: {...scope, requestId: preview.requestId, contextSnapshotId: preview.contextSnapshotId, model: preview.input.model, mode: "deepSeekNonThinking"}}) : null;
  const execute = vi.spyOn(runtime.execution.submission, "startWriting");
  const history = async () => (await runtime.history.gateway.readHistory({conversationId: scope.conversationId, afterSeq: 0, limit: 200})).events;
  const reopen = () => {
    const queueState = runtime.queue.snapshot(), execution = runtime.execution.snapshot(), events = agent.history.snapshot();
    const nextAgent = new WritingDiffDemoAgentStorage(files, undefined), next = createWritingDiffDemoRuntime(files, nextAgent, scope.manuscriptId, scope.conversationId);
    next.execution.historyOnly = true; nextAgent.history.restore(events); next.execution.restore(execution); next.queue.restore(queueState); cleanups.push(() => next.dispose());
    return next;
  };
  return {scope, inputId, tailId, files, agent, runtime, preparedRequest, preview, start, execute, history, reopen};
}
function startOf(f: Awaited<ReturnType<typeof fixture>>): AgentWritingQueuedStartRequest {if (!f.start) throw new Error("fixture has no prepared start"); return f.start;}
it("a failure before admission preserves the typed prepared claim and readonly status never starts a model", async () => {
  const f = await fixture(), request = startOf(f), original = await f.runtime.queue.gateway.read(f.scope);
  f.runtime.queue.failNextStart("beforeAdmission"); await expect(f.runtime.queue.preview.start(request)).rejects.toBe("AGENT_PREVIEW_SYNTHETIC_IO_BEFORE_COMMIT");
  const status = decodeAgentWritingQueuedStartStatusResponse(await f.runtime.queue.preview.startStatus(request));
  expect(status).toMatchObject({intent: request, status: {type: "prepared", preview: f.preview}});
  expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(original); expect(f.execute).not.toHaveBeenCalled(); expect(await f.history()).toHaveLength(0);
});
it("captures the complete first client start before awaits and persists admission identity across reopen", async () => {
  const f = await fixture({paused: true}), original = structuredClone(startOf(f)), caller = structuredClone(original);
  f.runtime.queue.failNextStart("afterAdmission"); const work = f.runtime.queue.preview.start(caller);
  (caller as {expectedRevision: number}).expectedRevision = 99; (caller.start as {mode: string}).mode = "chatGptDefault";
  await expect(work).rejects.toBe("AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST");
  expect(f.runtime.queue.snapshot().starts.get(original.start.requestId)?.intent).toEqual(original);
  expect(await f.history()).toHaveLength(0); expect(f.execute).not.toHaveBeenCalled();
  const admitted = await f.runtime.queue.gateway.read(f.scope);
  expect(admitted).toMatchObject({revision: original.expectedRevision + 2, paused: false, preparingInputId: null, inputs: [{inputId: f.tailId, paused: true}]});
  const next = f.reopen(), execute = vi.spyOn(next.execution.submission, "startWriting"), reopened = await next.queue.gateway.read(f.scope);
  expect((await next.queue.preview.startStatus(original)).status).toMatchObject({type: "admitted", preview: f.preview});
  expect(execute).not.toHaveBeenCalled(); expect(await next.queue.preview.recover(f.scope)).toBeNull();
  await expect(next.queue.preview.start(original)).rejects.toMatchObject({code: "recoveryRequired"}); expect(await next.queue.gateway.read(f.scope)).toEqual(reopened);
  const resumed = await next.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: reopened.revision, edit: {type: "resume"}});
  await next.queue.preview.start(original); expect(execute).toHaveBeenCalledExactlyOnceWith(original.start);
  expect(await next.queue.gateway.read(f.scope)).toEqual(resumed); expect((await next.queue.preview.startStatus(original)).status.type).toBe("started");
});
it("a lost ACK after real TurnStarted is inspectable without another start or a current queue pointer", async () => {
  const f = await fixture(), request = startOf(f); f.runtime.queue.failNextStart("afterStarted");
  await expect(f.runtime.queue.preview.start(request)).rejects.toBe("AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST");
  expect(await f.runtime.queue.preview.recover(f.scope)).toBeNull();
  expect(decodeAgentWritingQueuedStartStatusResponse(await f.runtime.queue.preview.startStatus(request)).status).toMatchObject({type: "started", result: {alreadyStarted: true, outcome: null, requestId: request.start.requestId}});
  expect((await f.history()).filter(event => event.payload.type === "turnStarted")).toHaveLength(1); expect(f.execute).toHaveBeenCalledOnce();
  const reopened = f.reopen(), execute = vi.spyOn(reopened.execution.submission, "startWriting"), queue = await reopened.queue.gateway.read(f.scope);
  expect((await reopened.queue.preview.startStatus(request)).status).toMatchObject({type: "started", result: {outcome: null}});
  expect(await reopened.queue.preview.start(request)).toMatchObject({alreadyStarted: true, outcome: null});
  expect(await reopened.queue.gateway.read(f.scope)).toEqual(queue); expect(queue.paused).toBe(true); expect(execute).not.toHaveBeenCalled();
});
it("started history remains readable when the current execution profile retires, while prepared status cannot grant that profile", async () => {
  const started = await fixture(), request = startOf(started); await started.runtime.queue.preview.start(request);
  const prepared = await fixture(); vi.spyOn(CoreAgent, "executionModeForModel").mockReturnValue(null);
  expect((await started.runtime.queue.preview.startStatus(request)).status).toMatchObject({type: "started", result: {alreadyStarted: true}});
  expect((await prepared.runtime.queue.preview.startStatus(startOf(prepared))).status).toMatchObject({type: "unconfirmed", detail: {safeCause: "queue_start_mode_mismatch"}});
  expect(await started.runtime.queue.preview.start(request)).toMatchObject({alreadyStarted: true}); expect(started.execute).toHaveBeenCalledOnce();
});
it("completed history survives missing execution copy and unavailable model route, preserving a later Pause on replay", async () => {
  const f = await fixture(), request = startOf(f); await f.runtime.queue.preview.start(request); await f.runtime.execution.finishScenario("completed");
  expect(await f.runtime.queue.preview.start(request)).toMatchObject({alreadyStarted: true, outcome: "completed"});
  const snapshot = f.runtime.execution.snapshot().map(([key, value]) => {const {workspace: _workspace, buffers: _buffers, ...record} = value; return [key, record] as const;});
  f.runtime.execution.restore(snapshot); f.runtime.queue.restore(f.runtime.queue.snapshot()); f.runtime.queue.failNextStart("unavailable");
  const current = await f.runtime.queue.gateway.read(f.scope), validate = vi.spyOn(f.runtime.execution, "validateQueuedStart"), workspace = vi.spyOn(f.runtime.gateways.workspace, "restore");
  const status = decodeAgentWritingQueuedStartStatusResponse(await f.runtime.queue.preview.startStatus(request));
  expect(status.status).toMatchObject({type: "started", result: {alreadyStarted: true, outcome: "completed"}});
  expect(await f.runtime.queue.preview.start(request)).toMatchObject({alreadyStarted: true, outcome: "completed"});
  expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(current); expect(current.paused).toBe(true);
  expect(validate).not.toHaveBeenCalled(); expect(workspace).not.toHaveBeenCalled(); expect(f.execute).toHaveBeenCalledOnce();
});
it("execution flags alone do not prove history, and a missing plan cannot silently start an admitted request", async () => {
  const f = await fixture(), request = startOf(f); f.runtime.queue.failNextStart("afterAdmission"); await expect(f.runtime.queue.preview.start(request)).rejects.toBeDefined();
  const snapshot = f.runtime.execution.snapshot().map(([key, value]) => {const {workspace: _workspace, buffers: _buffers, ...record} = value; return [key, {...record, started: true, outcome: "canceled" as const}] as const;});
  f.runtime.execution.restore(snapshot);
  expect((await f.runtime.queue.preview.startStatus(request)).status.type).toBe("admitted"); expect(await f.history()).toHaveLength(0);
  const queue = await f.runtime.queue.gateway.read(f.scope); await expect(f.runtime.queue.preview.start(request)).rejects.toMatchObject({code: "confirmationMismatch"});
  expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(queue); expect(f.execute).not.toHaveBeenCalled();
});
it("status reader unavailability is safe Unconfirmed, with no write or execution", async () => {
  const f = await fixture(), request = startOf(f), queue = await f.runtime.queue.gateway.read(f.scope); f.runtime.queue.failStartStatus(true);
  const response = decodeAgentWritingQueuedStartStatusResponse(await f.runtime.queue.preview.startStatus(request));
  expect(response.status).toEqual({type: "unconfirmed", detail: {operationId: request.start.requestId, epoch: null, sqlFence: null,
    phase: "queue-start-status", sqliteCode: null, sqliteExtendedCode: null, safeCause: "mock_start_reader_unavailable"}});
  expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(queue); expect(f.execute).not.toHaveBeenCalled();
  f.runtime.queue.failStartStatus(false); expect((await f.runtime.queue.preview.startStatus(request)).status.type).toBe("prepared");
});
it("an admitted R rejects every changed first-start identity, including mode and model revision", async () => {
  const f = await fixture(), request = startOf(f); f.runtime.queue.failNextStart("afterAdmission"); await expect(f.runtime.queue.preview.start(request)).rejects.toBeDefined();
  const changes: AgentWritingQueuedStartRequest[] = [{...request, inputId: crypto.randomUUID()}, {...request, expectedRevision: request.expectedRevision + 1},
    ...[{conversationId: crypto.randomUUID()}, {manuscriptId: crypto.randomUUID()}, {contextSnapshotId: crypto.randomUUID()}, {mode: "chatGptDefault" as const},
      {model: {...request.start.model, revision: "changed"}}].map(patch => ({...request, start: {...request.start, ...patch}}))];
  const queue = await f.runtime.queue.gateway.read(f.scope);
  for (const changed of changes) await expect(f.runtime.queue.preview.start(changed)).rejects.toMatchObject({code: "conflict"});
  expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(queue); expect(f.execute).not.toHaveBeenCalled();
});
it("admission origins cannot cross cancellation or Compact purposes", async () => {
  const f = await fixture(), request = startOf(f); f.runtime.queue.failNextStart("afterAdmission"); await expect(f.runtime.queue.preview.start(request)).rejects.toBeDefined();
  const cancel = {...f.scope, requestId: request.start.requestId, expectedRevision: request.expectedRevision, inputId: f.inputId, previewRequestId: request.start.requestId};
  await expect(f.runtime.queue.preview.cancel(cancel)).rejects.toMatchObject({code: "storage"}); await expect(f.runtime.queue.preview.cancelStatus(cancel)).rejects.toMatchObject({code: "storage"});
  const compact = await fixture({prepared: false, text: "/compact"});
  const result = await compact.runtime.compaction.gateway.prepareWritingQueued({...compact.preparedRequest, expectedQueueRevision: compact.preparedRequest.expectedRevision,
    previousRequestId: null, expectedRevision: 0});
  const ordinary = decodeAgentWritingQueuedStartRequest({inputId: compact.inputId, expectedRevision: result.queue.revision,
    start: {...compact.scope, requestId: result.preview.requestId, contextSnapshotId: result.preview.contextSnapshotId, model: result.preview.input.model, mode: "deepSeekNonThinking"}});
  expect((await compact.runtime.queue.preview.startStatus(ordinary)).status).toMatchObject({type: "unconfirmed", detail: {safeCause: "queue_start_purpose_mismatch"}});
  await expect(compact.runtime.queue.preview.start(ordinary)).rejects.toBeDefined(); expect(compact.execute).not.toHaveBeenCalled(); expect(await compact.history()).toHaveLength(0);
});
it("a missing request reports NotFound, while legacy or inconsistent evidence remains Unconfirmed", async () => {
  const f = await fixture(), request = startOf(f), missing = {...request, inputId: crypto.randomUUID(), start: {...request.start, requestId: crypto.randomUUID()}};
  expect((await f.runtime.queue.preview.startStatus(missing)).status.type).toBe("notFound");
  const snapshot = f.runtime.queue.snapshot(); snapshot.preparations.clear(); f.runtime.queue.restore(snapshot);
  expect((await f.runtime.queue.preview.startStatus(request)).status).toMatchObject({type: "unconfirmed", detail: {safeCause: "queue_start_legacy_preparation_unconfirmed"}});
  expect(f.execute).not.toHaveBeenCalled();
});
