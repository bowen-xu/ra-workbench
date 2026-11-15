import { afterEach, expect, it, vi } from "vitest";
import {
  agentWritingCompactionStartRequest, decodeAgentSubmissionPreview, decodeAgentWritingQueuedCompactionPrepareRequest,
  type AgentWritingPrepareRequest, type AgentWritingQueuedCompactionPrepareRequest,
  type AgentWritingQueuedCompactionPreview, type AgentWritingQueuedCompactionSettleRequest,
} from "@research-assistant/core/agent";
import { MockAgentCommandGateway } from "../../../core/features/agent/MockAgentCommandGateway";
import { MockWritingCompactionGateway } from "./MockWritingCompactionGateway";
import { WritingDiffDemoHistory } from "./WritingDiffDemoHistory";
import { WritingDiffDemoQueue } from "./WritingDiffDemoQueue";

const id = (n: number) => `12345678-1234-5678-abcd-${String(n).padStart(12, "0")}`;
const scope = { conversationId: id(1), manuscriptId: id(2) };
const startRequest = ({ preview, expectedRevision }: AgentWritingQueuedCompactionPreview) => agentWritingCompactionStartRequest({ preview, expectedRevision });
async function fixture() {
  vi.useFakeTimers();
  const history = new WritingDiffDemoHistory();
  const submission = { prepareWriting: vi.fn(async (request: AgentWritingPrepareRequest) => decodeAgentSubmissionPreview({
    schemaVersion: 1, ...scope, requestId: request.requestId, turnId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID(),
    input: { text: request.text, userInput: request.userInput ?? [], model: { providerId: "fixture", modelId: "model", revision: null }, reasoningEffort: null, context: [] },
  })), prepareWritingStatus: vi.fn(async (): Promise<never> => {throw new Error("ordinary preparation status is not part of this queued fixture");}),
    startStatus: vi.fn(async (): Promise<never> => {throw new Error("ordinary start status is not part of this queued fixture");}), restoreWriting: vi.fn(), startWriting: vi.fn() };
  const queue = new WritingDiffDemoQueue(submission), commands = new MockAgentCommandGateway(100);
  const preparation = { materialize: async (request: AgentWritingPrepareRequest) => ({
    preview: await submission.prepareWriting(request), commit: (accept: () => void) => accept(),
  }) };
  const compact = vi.spyOn(commands, "compact"), native = new MockWritingCompactionGateway(preparation, history, queue, commands);
  const edit = async (edit: Parameters<typeof queue.gateway.edit>[0]["edit"]) => queue.gateway.edit({ ...scope, requestId: crypto.randomUUID(),
    expectedRevision: (await queue.gateway.read(scope)).revision, edit });
  await edit({ type: "add", inputId: id(3), text: "/compact" });
  const request = async (requestId = id(4), previousRequestId: string | null = null): Promise<AgentWritingQueuedCompactionPrepareRequest> => ({
    inputId: id(3), expectedQueueRevision: (await queue.gateway.read(scope)).revision, previousRequestId,
    expectedRevision: (await history.gateway.open(scope.conversationId)).watermark,
    submission: { conversationId: scope.conversationId, requestId, text: "/compact", context: { manuscriptId: scope.manuscriptId, files: [] } },
  });
  const settle = async (prepared: AgentWritingQueuedCompactionPreview, operationId = crypto.randomUUID()): Promise<AgentWritingQueuedCompactionSettleRequest> => ({
    ...scope, operationId, inputId: prepared.inputId, requestId: prepared.requestId, claimRevision: prepared.claimRevision,
    expectedQueueRevision: (await queue.gateway.read(scope)).revision,
  });
  return { history, submission, preparation, queue, commands, compact, native, edit, request, settle };
}
afterEach(() => vi.useRealTimers());

it("replays an older sparse complete intent after canonical defaults without rewriting its frozen receipt", async () => {
  const f = await fixture(), request = await f.request(), prepared = await f.native.gateway.prepareWritingQueued(request);
  const snapshot = f.native.snapshot();
  for (const [, record] of snapshot.records) {
    const intent = JSON.parse(record.intent) as {request: {request: AgentWritingPrepareRequest}; queued: AgentWritingQueuedCompactionPrepareRequest};
    for (const submission of [intent.request.request, intent.queued.submission]) for (const key of ["userInput", "resources", "model", "retrieval", "sessionConfig", "turnConfig"] as const) delete (submission as unknown as Record<string, unknown>)[key];
    (record as {intent: string}).intent = JSON.stringify(intent);
  }
  f.native.restore(snapshot);
  expect(await f.native.gateway.prepareWritingQueued(decodeAgentWritingQueuedCompactionPrepareRequest(request))).toEqual(prepared);
  await expect(f.native.gateway.prepareWritingQueued({...request, submission: {...request.submission, text: "changed"}})).rejects.toMatchObject({code: "confirmationMismatch"});
  expect(f.submission.prepareWriting).toHaveBeenCalledOnce();
  f.native.dispose(); f.commands.dispose(); f.history.dispose();
});

it("retains I at start, preserves concurrent tails, and settles only a completed Compact turn", async () => {
  const f = await fixture(), prepared = await f.native.gateway.prepareWritingQueued(await f.request());
  await f.native.gateway.start(startRequest(prepared));
  expect((await f.queue.gateway.read(scope)).preparingInputId).toBe(id(3));
  await f.edit({ type: "add", inputId: id(8), text: "later input" });
  await expect(f.native.gateway.settleWritingQueued(await f.settle(prepared))).rejects.toMatchObject({ code: "confirmationMismatch" });
  await vi.advanceTimersByTimeAsync(100);
  const result = await f.native.gateway.settleWritingQueued(await f.settle(prepared));
  expect(result).toMatchObject({ outcome: "completed", queue: { preparingInputId: null, paused: false, inputs: [{ inputId: id(8) }] } });
  const events = (await f.history.gateway.readHistory({ conversationId: scope.conversationId, afterSeq: 0, limit: 100 })).events;
  expect(events[0]?.payload).toMatchObject({ type: "turnStarted", taskKind: "compact" });
  expect(events.filter(event => event.payload.type === "itemStarted").map(event => event.payload)).toEqual([
    expect.objectContaining({ kind: { type: "contextCompaction" } }),
  ]);
  expect(f.submission.startWriting).not.toHaveBeenCalled();
  f.native.dispose(); f.commands.dispose(); f.history.dispose();
});

it("same failed R never reruns; explicit R2 reclaims I and stale settlement cannot consume it", async () => {
  const f = await fixture(); f.native.nextOutcome = "failed";
  const first = await f.native.gateway.prepareWritingQueued(await f.request()), firstStart = startRequest(first);
  await f.native.gateway.start(firstStart); await vi.advanceTimersByTimeAsync(100);
  expect(await f.native.gateway.start(firstStart)).toMatchObject({ alreadyStarted: true, outcome: "failed" });
  expect(f.compact).toHaveBeenCalledTimes(1);
  const oldSettle = await f.settle(first), released = await f.native.gateway.settleWritingQueued(oldSettle);
  expect(released.queue).toMatchObject({ paused: true, preparingInputId: null, inputs: [{ inputId: id(3) }] });
  await f.edit({ type: "resumeInput", inputId: id(3) });
  const second = await f.native.gateway.prepareWritingQueued(await f.request(id(5), first.requestId));
  const claimed = await f.queue.gateway.read(scope);
  expect(await f.native.gateway.settleWritingQueued(oldSettle)).toEqual(released);
  expect(await f.queue.gateway.read(scope)).toEqual(claimed);
  await expect(f.native.gateway.settleWritingQueued({ ...oldSettle, operationId: id(99), expectedQueueRevision: claimed.revision })).rejects.toMatchObject({ code: "confirmationMismatch" });
  expect(await f.native.gateway.readWritingQueued({ ...scope, inputId: id(3), requestId: first.requestId })).toMatchObject({ requestId: first.requestId, current: false, outcome: "failed" });
  await f.native.gateway.start(startRequest(second)); await vi.advanceTimersByTimeAsync(100);
  expect((await f.native.gateway.settleWritingQueued(await f.settle(second))).outcome).toBe("completed");
  expect(f.compact).toHaveBeenCalledTimes(2);
  f.native.dispose(); f.commands.dispose(); f.history.dispose();
});

it("lost prepare, start and settle acknowledgments retain their original durable identities", async () => {
  const f = await fixture(), intent = await f.request();
  f.native.failNext("prepare", "afterCommit");
  await expect(f.native.gateway.prepareWritingQueued(intent)).rejects.toThrow("lost queued");
  const prepared = await f.native.gateway.prepareWritingQueued(intent);
  expect(f.submission.prepareWriting).toHaveBeenCalledTimes(1);
  await f.edit({ type: "add", inputId: id(8), text: "tail" });
  expect(await f.native.gateway.prepareWritingQueued(intent)).toEqual(prepared);
  f.native.failNext("start", "afterCommit");
  await expect(f.native.gateway.start(startRequest(prepared))).rejects.toThrow("lost compaction start");
  expect(await f.native.gateway.start(startRequest(prepared))).toMatchObject({ alreadyStarted: true });
  await vi.advanceTimersByTimeAsync(100);
  const settlement = await f.settle(prepared);
  f.native.failNext("settle", "afterCommit");
  await expect(f.native.gateway.settleWritingQueued(settlement)).rejects.toThrow("lost queued compaction settlement");
  const after = await f.queue.gateway.read(scope);
  expect(await f.native.gateway.settleWritingQueued(settlement)).toMatchObject({ outcome: "completed", queue: after });
  expect(f.compact).toHaveBeenCalledTimes(1);
  f.native.dispose(); f.commands.dispose(); f.history.dispose();
});

it("reopens a started compact as interrupted, retaining its claim for release without sampling", async () => {
  const f = await fixture(), prepared = await f.native.gateway.prepareWritingQueued(await f.request());
  await f.native.gateway.start(startRequest(prepared));
  const history = f.history.snapshot(), queue = f.queue.snapshot(), compact = f.native.snapshot();
  f.native.dispose(); f.commands.dispose(); f.history.dispose();
  const reopenedHistory = new WritingDiffDemoHistory(); reopenedHistory.restore(history);
  const reopenedQueue = new WritingDiffDemoQueue(f.submission); reopenedQueue.restore(queue);
  const commands = new MockAgentCommandGateway(100), provider = vi.spyOn(commands, "compact");
  const reopened = new MockWritingCompactionGateway(f.preparation, reopenedHistory, reopenedQueue, commands); reopened.restore(compact);
  const state = await reopened.gateway.readWritingQueued({ ...scope, inputId: id(3), requestId: prepared.requestId });
  expect(state).toMatchObject({ kind: "compact", outcome: "interrupted", current: true, queue: { paused: true, preparingInputId: id(3) } });
  const result = await reopened.gateway.settleWritingQueued({ ...await f.settle(prepared), expectedQueueRevision: (await reopenedQueue.gateway.read(scope)).revision });
  expect(result).toMatchObject({ outcome: "interrupted", queue: { paused: true, preparingInputId: null, inputs: [{ inputId: id(3) }] } });
  expect(provider).not.toHaveBeenCalled();
  reopened.dispose(); commands.dispose(); reopenedHistory.dispose();
});

it("cancels through the existing turn port and keeps the queued payload", async () => {
  const f = await fixture(), prepared = await f.native.gateway.prepareWritingQueued(await f.request());
  await f.native.gateway.start(startRequest(prepared));
  expect(await f.native.interrupt({ conversationId: scope.conversationId, turnId: prepared.preview.turnId })).toMatchObject({ cancellationRequested: true });
  const result = await f.native.gateway.settleWritingQueued(await f.settle(prepared));
  expect(result).toMatchObject({ outcome: "canceled", queue: { paused: true, inputs: [{ inputId: id(3), text: "/compact" }] } });
  f.native.dispose(); f.commands.dispose(); f.history.dispose();
});

it("discovers ordinary prepared inputs by purpose instead of guessing from command text", async () => {
  const f = await fixture(), request = await f.request();
  const ordinary = await f.queue.preview.prepare({ inputId: request.inputId, expectedRevision: request.expectedQueueRevision, submission: request.submission });
  expect(await f.native.gateway.readWritingQueued({ ...scope, inputId: id(3), requestId: null })).toMatchObject({ kind: "submission", preview: ordinary });
  await expect(f.native.gateway.prepareWritingQueued({ ...request, submission: { ...request.submission, requestId: id(9) } })).rejects.toMatchObject({ code: "conflict" });
  expect(f.compact).not.toHaveBeenCalled();
  f.native.dispose(); f.commands.dispose(); f.history.dispose();
});

it("discovers the current ordinary preview before a released compact origin for the same I", async () => {
  const f = await fixture(); f.native.nextOutcome = "failed";
  const compact = await f.native.gateway.prepareWritingQueued(await f.request());
  await f.native.gateway.start(startRequest(compact)); await vi.advanceTimersByTimeAsync(100);
  await f.native.gateway.settleWritingQueued(await f.settle(compact));
  await f.edit({type: "update", inputId: id(3), text: "edited ordinary input"});
  await f.edit({type: "resumeInput", inputId: id(3)});
  const next = await f.request(id(5)), ordinary = await f.queue.preview.prepare({
    inputId: next.inputId, expectedRevision: next.expectedQueueRevision, submission: {...next.submission, text: "edited ordinary input"},
  });
  await f.edit({type: "add", inputId: id(8), text: "newer tail"});
  const current = {...ordinary, queue: await f.queue.gateway.read(scope)};
  expect(await f.native.gateway.readWritingQueued({...scope, inputId: id(3), requestId: null})).toMatchObject({kind: "submission", preview: current});
  expect(await f.native.gateway.readWritingQueued({...scope, inputId: id(3), requestId: ordinary.preview.requestId})).toMatchObject({kind: "submission", preview: current});
  expect(await f.native.gateway.readWritingQueued({...scope, inputId: id(3), requestId: compact.requestId}))
    .toMatchObject({kind: "compact", requestId: compact.requestId, outcome: "failed", current: false});
  f.native.dispose(); f.commands.dispose(); f.history.dispose();
});
