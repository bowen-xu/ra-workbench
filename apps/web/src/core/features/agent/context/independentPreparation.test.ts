import {expect, it, vi} from "vitest";
import {IndependentPreparation} from "./independentPreparation";
import type {IndependentQueueResume} from "./independentQueueResume";
import {independentQueueResume} from "./independentQueueResume";
import {AgentInputQueueController} from "@research-assistant/data/agent";
import type {AgentContextQueueEditRequest, AgentQueueState} from "@research-assistant/core/agent";

const id = (n: number) => `abcdef00-1234-4678-abcd-${String(n).padStart(12, "0")}`;
const request = {conversationId: id(1), requestId: id(2), text: "Question", resources: []};
const intent = {...request, userInput: [], retrieval: null, model: null, queue: null};
const detail = {operationId: id(2), epoch: id(3), sqlFence: "0", phase: "context-prepare-status", sqliteCode: null, sqliteExtendedCode: null, safeCause: "context_prepare_worker_unconfirmed"};
const result = {schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(4), contextSnapshotId: id(5), input: {text: "Question", model: {providerId: "fixture", modelId: "model", revision: null}, reasoningEffort: null, context: []}};
const proof = {schemaVersion: 1, operationId: id(2), conversationId: id(1), family: "agent.context.submission.prepare", epoch: id(3), sqlFence: "12", replayed: false, result: null};
function storage(): Storage {
  const entries = new Map<string, string>();
  return {get length() {return entries.size;}, key: index => [...entries.keys()][index] ?? null, getItem: key => entries.get(key) ?? null,
    setItem: (key, value) => {entries.set(key, String(value));}, removeItem: key => {entries.delete(key);}, clear: () => entries.clear()};
}
function fixture() {
  const saved = storage();
  const read = vi.fn().mockResolvedValue({schemaVersion: 1, intent, expectedEpoch: null, status: {type: "unconfirmed", detail}});
  const prepare = vi.fn().mockResolvedValue({type: "committed", proof, result});
  const start = vi.fn().mockResolvedValue({schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(4), alreadyStarted: false, outcome: null});
  const startRead = vi.fn();
  const open = () => new IndependentPreparation(saved, "pending", id(1), {read}, {prepare}, undefined, {start, read: startRead});
  return {saved, read, prepare, start, startRead, open};
}
it("persists the complete intent and original epoch before sending", async () => {
  const f = fixture();
  f.prepare.mockImplementationOnce(async () => {
    expect(JSON.parse(f.saved.getItem("pending")!)).toMatchObject({intent, epoch: id(3), committed: null, preserveDraft: true});
    return {type: "committed", proof, result};
  });
  const owner = f.open(); await owner.begin(request, true);
  expect(owner.getSnapshot()).toMatchObject({phase: "ready", record: {intent, epoch: id(3), committed: {proof, result}}});
  expect(f.prepare).toHaveBeenCalledExactlyOnceWith(intent, {expectedEpoch: id(3)});
});
it("recovers a lost reply by inspection without sending again", async () => {
  const f = fixture(); f.prepare.mockRejectedValueOnce(new Error("lost reply"));
  const owner = f.open(); await owner.begin(request, false); expect(owner.getSnapshot().phase).toBe("unknown");
  const reopened = f.open();
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent, expectedEpoch: id(3), status: {type: "committed", proof: {...proof, replayed: true}, result}});
  await reopened.check(); expect(reopened.getSnapshot()).toMatchObject({phase: "ready", record: {epoch: id(3), committed: {result}}});
  expect(f.prepare).toHaveBeenCalledOnce();
});
it("keeps unknown or conflicting results behind the recovery barrier", async () => {
  const f = fixture(); f.prepare.mockRejectedValueOnce(new Error("lost reply"));
  await f.open().begin(request, false); const reopened = f.open();
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent, expectedEpoch: id(3), status: {type: "unconfirmed", detail}});
  await reopened.check(); expect(reopened.getSnapshot().phase).toBe("unconfirmed");
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent, expectedEpoch: id(9), status: {type: "committed", proof: {...proof, epoch: id(9)}, result}});
  await reopened.check(); expect(reopened.getSnapshot().phase).toBe("unknown"); expect(f.prepare).toHaveBeenCalledOnce();
});
it("does not send when local storage changes or epoch cannot be fixed", async () => {
  const f = fixture(); f.read.mockImplementationOnce(async () => {f.saved.setItem("pending", "other owner"); return {schemaVersion: 1, intent, expectedEpoch: null, status: {type: "unconfirmed", detail}};});
  const owner = f.open(); await owner.begin(request, false);
  expect(owner.getSnapshot().phase).toBe("unknown"); expect(f.prepare).not.toHaveBeenCalled();
  expect(f.open().getSnapshot().phase).toBe("localError");
});
it("requires the original frozen proof after a successful preparation is reopened", async () => {
  const f = fixture(); await f.open().begin(request, false); const reopened = f.open();
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent, expectedEpoch: id(3), status: {type: "committed", proof: {...proof, sqlFence: "13", replayed: true}, result}});
  await reopened.check(); expect(reopened.getSnapshot().phase).toBe("unknown");
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent, expectedEpoch: id(3), status: {type: "committed", proof: {...proof, replayed: true}, result}});
  await reopened.check(); expect(reopened.getSnapshot().phase).toBe("ready"); expect(f.prepare).toHaveBeenCalledOnce();
});
it("persists the start before sending and recovers a lost response without restarting", async () => {
  const f = fixture(); const owner = f.open(); await owner.begin(request, false);
  f.read.mockResolvedValue({schemaVersion: 1, intent, expectedEpoch: id(3), status: {type: "committed", proof: {...proof, replayed: true}, result}});
  f.start.mockImplementationOnce(async () => {
    expect(JSON.parse(f.saved.getItem("pending")!)).toMatchObject({schemaVersion: 2, start: {request: {requestId: id(2)}, result: null}});
    throw new Error("lost start response");
  });
  await owner.start(); expect(owner.getSnapshot().phase).toBe("unknown");
  const reopened = f.open(), startRequest = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(5), queue: null, resumeQueue: false};
  f.startRead.mockResolvedValueOnce({schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "started", result: {schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(4), runId: id(7), alreadyStarted: true, outcome: null}}});
  await reopened.check(); expect(reopened.getSnapshot().phase).toBe("started"); expect(f.start).toHaveBeenCalledOnce();
  expect(JSON.parse(f.saved.getItem("pending")!).start.result.runId).toBe(id(7));
});
it("keeps an unconfirmed start behind the barrier and rejects changed preparation proof", async () => {
  const f = fixture(); const owner = f.open(); await owner.begin(request, false);
  f.read.mockResolvedValue({schemaVersion: 1, intent, expectedEpoch: id(3), status: {type: "committed", proof: {...proof, replayed: true}, result}});
  f.start.mockRejectedValueOnce(new Error("lost start response")); await owner.start();
  const reopened = f.open(), startRequest = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(5), queue: null, resumeQueue: false};
  f.startRead.mockResolvedValueOnce({schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "unconfirmed", detail}});
  await reopened.check(); expect(reopened.getSnapshot().phase).toBe("unconfirmed");
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent, expectedEpoch: id(3), status: {type: "committed", proof: {...proof, sqlFence: "13"}, result}});
  await reopened.check(); expect(reopened.getSnapshot().phase).toBe("unknown"); expect(f.start).toHaveBeenCalledOnce();
});
it("enriches an existing start with the recovered owner and rejects a different owner", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, false);
  f.read.mockResolvedValue({schemaVersion:1,intent,expectedEpoch:id(3),status:{type:"committed",proof:{...proof,replayed:true},result}});
  await owner.start();
  const reopened = f.open(), startRequest = {conversationId:id(1),requestId:id(2),contextSnapshotId:id(5),queue:null,resumeQueue:false};
  const status = (runId:string) => ({schemaVersion:1,intent:startRequest,expectedEpoch:id(3),status:{type:"started",result:{
    schemaVersion:1,conversationId:id(1),requestId:id(2),turnId:id(4),runId,alreadyStarted:true,outcome:null}}});
  f.startRead.mockResolvedValueOnce(status(id(7))); await reopened.check();
  expect(reopened.getSnapshot()).toMatchObject({phase:"started",record:{start:{result:{runId:id(7)}}}});
  f.startRead.mockResolvedValueOnce(status(id(8))); await reopened.check();
  expect(reopened.getSnapshot().phase).toBe("unknown");
  expect(JSON.parse(f.saved.getItem("pending")!).start.result.runId).toBe(id(7));
  expect(f.start).toHaveBeenCalledOnce();
});
it("retains a proven start when its task owner cannot be discovered", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, false);
  f.read.mockResolvedValue({schemaVersion:1,intent,expectedEpoch:id(3),status:{type:"committed",proof:{...proof,replayed:true},result}});
  f.start.mockRejectedValueOnce(new Error("lost response")); await owner.start();
  const startRequest = {conversationId:id(1),requestId:id(2),contextSnapshotId:id(5),queue:null,resumeQueue:false};
  const started = {schemaVersion:1,conversationId:id(1),requestId:id(2),turnId:id(4),alreadyStarted:true,outcome:null};
  f.startRead.mockResolvedValue({schemaVersion:1,intent:startRequest,expectedEpoch:id(3),status:{type:"startedUnobserved",result:started}});
  const reopened = f.open(); await reopened.check();
  expect(reopened.getSnapshot()).toMatchObject({phase:"unconfirmed",cause:{code:"AGENT_TASK_IDENTITY_UNAVAILABLE"},record:{start:{result:started}}});
  const consume = vi.fn(); await reopened.finish(consume); expect(consume).not.toHaveBeenCalled();
  const reopenedAgain = f.open(); await reopenedAgain.check();
  expect(reopenedAgain.getSnapshot().phase).toBe("unconfirmed");
  expect(f.start).toHaveBeenCalledOnce();
  f.startRead.mockResolvedValueOnce({schemaVersion:1,intent:startRequest,expectedEpoch:id(3),status:{type:"started",result:{...started,runId:id(7)}}});
  await reopenedAgain.check();
  expect(reopenedAgain.getSnapshot()).toMatchObject({phase:"started",record:{start:{result:{runId:id(7)}}}});
});
it("retains a known run identity when its owner is no longer discoverable", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, false);
  f.read.mockResolvedValue({schemaVersion:1,intent,expectedEpoch:id(3),status:{type:"committed",proof:{...proof,replayed:true},result}});
  f.start.mockResolvedValueOnce({schemaVersion:1,conversationId:id(1),requestId:id(2),turnId:id(4),runId:id(7),alreadyStarted:false,outcome:null});
  await owner.start();
  const startRequest = {conversationId:id(1),requestId:id(2),contextSnapshotId:id(5),queue:null,resumeQueue:false};
  f.startRead.mockResolvedValueOnce({schemaVersion:1,intent:startRequest,expectedEpoch:id(3),status:{type:"startedUnobserved",result:{
    schemaVersion:1,conversationId:id(1),requestId:id(2),turnId:id(4),alreadyStarted:true,outcome:null}}});
  const reopened = f.open(); await reopened.check();
  expect(reopened.getSnapshot()).toMatchObject({phase:"started",record:{start:{result:{runId:id(7)}}}});
});
it("accepts a durable owner settlement only for the proven original start", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, false);
  f.read.mockResolvedValue({schemaVersion:1,intent,expectedEpoch:id(3),status:{type:"committed",proof:{...proof,replayed:true},result}});
  f.start.mockRejectedValueOnce(new Error("lost response")); await owner.start();
  const startRequest = {conversationId:id(1),requestId:id(2),contextSnapshotId:id(5),queue:null,resumeQueue:false};
  f.startRead.mockResolvedValueOnce({schemaVersion:1,intent:startRequest,expectedEpoch:id(3),status:{type:"startedSettled",result:{
    schemaVersion:1,conversationId:id(1),requestId:id(2),turnId:id(4),alreadyStarted:true,outcome:"completed"}}});
  const reopened = f.open(); await reopened.check();
  expect(reopened.getSnapshot()).toMatchObject({phase:"started",ownerSettled:true});
  const consume = vi.fn(); await reopened.finish(consume);
  expect(consume).toHaveBeenCalledOnce(); expect(reopened.getSnapshot().ownerSettled).toBe(false);
  expect(f.start).toHaveBeenCalledOnce();
});
it("retains the saved run while updating a confirmed terminal outcome", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, false);
  f.read.mockResolvedValue({schemaVersion: 1, intent, expectedEpoch: id(3), status: {type: "committed", proof, result}});
  f.start.mockResolvedValueOnce({schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(4), runId: id(7), alreadyStarted: false, outcome: null});
  await owner.start();
  const startRequest = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(5), queue: null, resumeQueue: false};
  f.startRead.mockResolvedValueOnce({schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "startedSettled",
    result: {schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(4), alreadyStarted: true, outcome: "completed"}}});
  const reopened = f.open(); await reopened.check();
  expect(reopened.getSnapshot()).toMatchObject({phase: "started", ownerSettled: true, record: {start: {result: {runId: id(7), outcome: "completed"}}}});
});
it("retains a captured draft identity and releases only after acknowledged start", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, false, {text: "Question", input: "[]", version: "version"});
  f.read.mockResolvedValue({schemaVersion: 1, intent, expectedEpoch: id(3), status: {type: "committed", proof: {...proof, replayed: true}, result}});
  await owner.start(); expect(owner.getSnapshot().phase).toBe("started");
  const consume = vi.fn(); await owner.finish(consume);
  expect(consume).toHaveBeenCalledWith(expect.objectContaining({schemaVersion: 3, draft: {text: "Question", input: "[]", version: "version"}}));
  expect(owner.getSnapshot().phase).toBe("idle"); expect(f.saved.getItem("pending")).toBeNull();
});
it("retains the started request until asynchronous consumption is durable", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, false);
  f.read.mockResolvedValue({schemaVersion: 1, intent, expectedEpoch: id(3), status: {type: "committed", proof: {...proof, replayed: true}, result}});
  await owner.start();
  let release!: () => void;
  const preserved = new Promise<void>(resolve => { release = resolve; });
  const finishing = owner.finish(async () => { await preserved; });
  await Promise.resolve();
  expect(f.saved.getItem("pending")).not.toBeNull();
  release(); await finishing;
  expect(owner.getSnapshot().phase).toBe("idle"); expect(f.saved.getItem("pending")).toBeNull();
});
it("retains the started request when asynchronous consumption fails", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(request, false);
  f.read.mockResolvedValue({schemaVersion: 1, intent, expectedEpoch: id(3), status: {type: "committed", proof: {...proof, replayed: true}, result}});
  await owner.start();
  await owner.finish(async () => { throw new Error("preservation failed"); });
  expect(owner.getSnapshot()).toMatchObject({phase: "unknown", cause: new Error("preservation failed")});
  expect(f.saved.getItem("pending")).not.toBeNull(); expect(f.start).toHaveBeenCalledOnce();
});
it("clears only the native pre-transaction model rejection, retaining other uncertain failures", async () => {
  const f = fixture(), owner = f.open(); f.prepare.mockRejectedValueOnce({code: "modelUnavailable"});
  await owner.begin(request, false); expect(owner.getSnapshot()).toMatchObject({phase: "idle", record: null, cause: {code: "modelUnavailable"}});
  expect(f.saved.getItem("pending")).toBeNull();
  f.prepare.mockRejectedValueOnce({code: "modelUnavailable", reason: "unexpected"});
  await owner.begin(request, false);
  expect(owner.getSnapshot().phase).toBe("unknown"); expect(f.saved.getItem("pending")).not.toBeNull();
});
it("reports an admitted queue input without restarting the original worker", async () => {
  const f = fixture(), queuedRequest = {...request, queue: {inputId: id(6), expectedRevision: 4}},
    queuedIntent = {...intent, queue: queuedRequest.queue}, queuedPreview = {...result, queue: {inputId: id(6), expectedRevision: 5}};
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent: queuedIntent, expectedEpoch: null, status: {type: "unconfirmed", detail}});
  f.prepare.mockResolvedValueOnce({type: "committed", proof, result: queuedPreview});
  const owner = f.open(); await owner.begin(queuedRequest, true);
  f.read.mockResolvedValue({schemaVersion: 1, intent: queuedIntent, expectedEpoch: id(3), status: {type: "committed", proof: {...proof, replayed: true}, result: queuedPreview}});
  f.start.mockRejectedValueOnce(new Error("lost start response")); await owner.start();
  const reopened = f.open(), startRequest = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(5), queue: queuedPreview.queue, resumeQueue: false};
  f.startRead.mockResolvedValueOnce({schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "admitted", preview: queuedPreview}});
  await reopened.check(); expect(reopened.getSnapshot().phase).toBe("admitted"); expect(f.start).toHaveBeenCalledOnce();
  await reopened.start(); expect(f.start).toHaveBeenCalledOnce();
  const status = {schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "admitted", preview: queuedPreview}};
  f.startRead.mockResolvedValue(status);
  await reopened.continueAdmitted(async () => ({conversationId: id(1), paused: true}));
  expect(reopened.getSnapshot().phase).toBe("unknown"); expect(f.start).toHaveBeenCalledOnce();
  await reopened.check(); expect(reopened.getSnapshot().phase).toBe("admitted");
  await reopened.continueAdmitted(async () => ({conversationId: id(1), paused: false}));
  expect(reopened.getSnapshot().phase).toBe("started");
  expect(f.start).toHaveBeenLastCalledWith(startRequest, id(3));
  expect(f.start).toHaveBeenCalledTimes(2);
});
it("finishes a settled legacy queued turn without resending or inventing an admission receipt", async () => {
  const f = fixture(), queuedRequest = {...request, queue: {inputId: id(6), expectedRevision: 4}},
    queuedIntent = {...intent, queue: queuedRequest.queue}, queuedPreview = {...result, queue: {inputId: id(6), expectedRevision: 5}};
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent: queuedIntent, expectedEpoch: null, status: {type: "unconfirmed", detail}});
  f.prepare.mockResolvedValueOnce({type: "committed", proof, result: queuedPreview});
  const owner = f.open(); await owner.begin(queuedRequest, true);
  f.read.mockResolvedValue({schemaVersion: 1, intent: queuedIntent, expectedEpoch: id(3), status: {type: "committed", proof, result: queuedPreview}});
  f.start.mockRejectedValueOnce(new Error("lost reply")); await owner.start();
  const startRequest = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(5), queue: queuedPreview.queue, resumeQueue: false};
  f.startRead.mockResolvedValueOnce({schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "legacyStartedSettled",
    result: {schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(4), alreadyStarted: true, outcome: "completed"}}});
  const reopened = f.open(); await reopened.check();
  expect(reopened.getSnapshot()).toMatchObject({phase: "started", ownerSettled: true, record: {start: {result: {outcome: "completed"}}}});
  const consume = vi.fn(); await reopened.finish(consume);
  expect(consume).toHaveBeenCalledOnce(); expect(f.start).toHaveBeenCalledOnce();
  expect(f.saved.getItem("pending")).toBeNull();
});
it("recovers a legacy running queue turn through its original task owner without resending", async () => {
  const f = fixture(), queuedRequest = {...request, queue: {inputId: id(6), expectedRevision: 4}},
    queuedIntent = {...intent, queue: queuedRequest.queue}, queuedPreview = {...result, queue: {inputId: id(6), expectedRevision: 5}};
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent: queuedIntent, expectedEpoch: null, status: {type: "unconfirmed", detail}});
  f.prepare.mockResolvedValueOnce({type: "committed", proof, result: queuedPreview});
  const owner = f.open(); await owner.begin(queuedRequest, true);
  f.read.mockResolvedValue({schemaVersion: 1, intent: queuedIntent, expectedEpoch: id(3), status: {type: "committed", proof, result: queuedPreview}});
  f.start.mockRejectedValueOnce(new Error("lost reply")); await owner.start();
  const startRequest = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(5), queue: queuedPreview.queue, resumeQueue: false};
  f.startRead.mockResolvedValueOnce({schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "legacyStartedObserved",
    result: {schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(4), runId: id(7), alreadyStarted: true, outcome: null}}});
  const reopened = f.open(); await reopened.check();
  expect(reopened.getSnapshot()).toMatchObject({phase: "started", ownerSettled: false, record: {start: {result: {runId: id(7)}}}});
  expect(f.start).toHaveBeenCalledOnce();
});
it("does not resend if the original worker starts during explicit continuation", async () => {
  const f = fixture(), queuedRequest = {...request, queue: {inputId: id(6), expectedRevision: 4}},
    queuedIntent = {...intent, queue: queuedRequest.queue}, queuedPreview = {...result, queue: {inputId: id(6), expectedRevision: 5}},
    startRequest = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(5), queue: queuedPreview.queue, resumeQueue: false};
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent: queuedIntent, expectedEpoch: null, status: {type: "unconfirmed", detail}});
  f.prepare.mockResolvedValueOnce({type: "committed", proof, result: queuedPreview});
  const owner = f.open(); await owner.begin(queuedRequest, true);
  f.read.mockResolvedValue({schemaVersion: 1, intent: queuedIntent, expectedEpoch: id(3), status: {type: "committed", proof, result: queuedPreview}});
  f.start.mockRejectedValueOnce(new Error("lost reply")); await owner.start();
  f.startRead.mockResolvedValueOnce({schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "admitted", preview: queuedPreview}});
  await owner.check();
  f.startRead.mockResolvedValueOnce({schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "started",
    result: {schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(4), alreadyStarted: true, outcome: null}}});
  const readQueue = vi.fn(); await owner.continueAdmitted(readQueue);
  expect(owner.getSnapshot().phase).toBe("started"); expect(readQueue).not.toHaveBeenCalled(); expect(f.start).toHaveBeenCalledOnce();
});

it("retains an unknown resume even if the original worker starts, and recovers both receipts after reopen", async () => {
  const f = fixture(), queuedRequest = {...request, queue: {inputId: id(6), expectedRevision: 4}},
    queuedIntent = {...intent, queue: queuedRequest.queue}, queuedPreview = {...result, queue: {inputId: id(6), expectedRevision: 5}},
    startRequest = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(5), queue: queuedPreview.queue, resumeQueue: false},
    resume = {conversationId: id(1), requestId: id(7), expectedRevision: 9, expectedEpoch: id(3), edit: {type: "resume" as const}};
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent: queuedIntent, expectedEpoch: null, status: {type: "unconfirmed", detail}});
  f.prepare.mockResolvedValueOnce({type: "committed", proof, result: queuedPreview});
  const owner = f.open(); await owner.begin(queuedRequest, true);
  f.read.mockResolvedValue({schemaVersion: 1, intent: queuedIntent, expectedEpoch: id(3), status: {type: "committed", proof, result: queuedPreview}});
  f.start.mockRejectedValueOnce(new Error("lost start")); await owner.start();
  const admitted = {schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "admitted", preview: queuedPreview}};
  f.startRead.mockResolvedValue(admitted); await owner.check();
  const paused = {schemaVersion: 1 as const, conversationId: id(1), revision: 9, paused: true, preparingInputId: null, inputs: []};
  const queue = {read: vi.fn(async () => paused), resume: vi.fn<IndependentQueueResume["resume"]>(async (_c, _e, _original, save) => {await save(resume); return "unknown";})};
  await owner.continuePaused(queue); expect(owner.getSnapshot().phase).toBe("resumePending");
  expect(JSON.parse(f.saved.getItem("pending")!).resume).toEqual({request: resume, acknowledged: false});
  const saved = f.saved.getItem("pending")!, altered = JSON.parse(saved); altered.resume.request.expectedEpoch = id(8);
  f.saved.setItem("pending", JSON.stringify(altered)); expect(f.open().getSnapshot().phase).toBe("localError"); f.saved.setItem("pending", saved);
  const started = {schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "started", result:
    {schemaVersion: 1, conversationId: id(1), requestId: id(2), turnId: id(4), alreadyStarted: true, outcome: null}}};
  f.startRead.mockResolvedValue(started); const reopened = f.open(); await reopened.check();
  expect(reopened.getSnapshot().phase).toBe("resumePending");
  const consume = vi.fn(); await reopened.finish(consume); expect(consume).not.toHaveBeenCalled();
  queue.resume.mockImplementationOnce(async (_c, _e, original, save) => {expect(original).toEqual(resume); await save(resume); return "acknowledged";});
  await reopened.continuePaused(queue); expect(reopened.getSnapshot().phase).toBe("started");
  await reopened.finish(consume); expect(consume).toHaveBeenCalledOnce(); expect(f.start).toHaveBeenCalledOnce();
});
it("does not start or undo a later pause after the original resume was acknowledged", async () => {
  const f = fixture(), queuedRequest = {...request, queue: {inputId: id(6), expectedRevision: 4}},
    queuedIntent = {...intent, queue: queuedRequest.queue}, queuedPreview = {...result, queue: {inputId: id(6), expectedRevision: 5}},
    startRequest = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(5), queue: queuedPreview.queue, resumeQueue: false},
    resume = {conversationId: id(1), requestId: id(7), expectedRevision: 9, expectedEpoch: id(3), edit: {type: "resume" as const}};
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent: queuedIntent, expectedEpoch: null, status: {type: "unconfirmed", detail}});
  f.prepare.mockResolvedValueOnce({type: "committed", proof, result: queuedPreview});
  const owner = f.open(); await owner.begin(queuedRequest, true);
  f.read.mockResolvedValue({schemaVersion: 1, intent: queuedIntent, expectedEpoch: id(3), status: {type: "committed", proof, result: queuedPreview}});
  f.start.mockRejectedValueOnce(new Error("lost start")); await owner.start();
  f.startRead.mockResolvedValue({schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "admitted", preview: queuedPreview}});
  await owner.check();
  const paused = {schemaVersion: 1 as const, conversationId: id(1), revision: 11, paused: true, preparingInputId: null, inputs: []};
  const queue = {read: vi.fn(async () => paused), resume: vi.fn<IndependentQueueResume["resume"]>(async (_c, _e, _original, save) => {await save(resume); return "acknowledged";})};
  await owner.continuePaused(queue);
  expect(owner.getSnapshot()).toMatchObject({phase: "admitted", record: {resume: null}});
  expect(f.start).toHaveBeenCalledOnce(); expect(queue.resume).toHaveBeenCalledOnce();
});
it("continues a paused admission through the real queue owner with the original data epoch", async () => {
  const f = fixture(), queuedRequest = {...request, queue: {inputId: id(6), expectedRevision: 4}},
    queuedIntent = {...intent, queue: queuedRequest.queue}, queuedPreview = {...result, queue: {inputId: id(6), expectedRevision: 5}},
    startRequest = {conversationId: id(1), requestId: id(2), contextSnapshotId: id(5), queue: queuedPreview.queue, resumeQueue: false};
  f.read.mockResolvedValueOnce({schemaVersion: 1, intent: queuedIntent, expectedEpoch: null, status: {type: "unconfirmed", detail}});
  f.prepare.mockResolvedValueOnce({type: "committed", proof, result: queuedPreview});
  const owner = f.open(); await owner.begin(queuedRequest, true);
  f.read.mockResolvedValue({schemaVersion: 1, intent: queuedIntent, expectedEpoch: id(3), status: {type: "committed", proof, result: queuedPreview}});
  f.start.mockRejectedValueOnce(new Error("lost start")); await owner.start();
  f.startRead.mockResolvedValue({schemaVersion: 1, intent: startRequest, expectedEpoch: id(3), status: {type: "admitted", preview: queuedPreview}});
  await owner.check();
  let state: AgentQueueState = {schemaVersion: 1, conversationId: id(1), revision: 9, paused: true, preparingInputId: null, inputs: []};
  const read = vi.fn(async () => state), edit = vi.fn(async (value: AgentContextQueueEditRequest) => {
    expect(value).toEqual({conversationId: id(1), requestId: id(7), expectedRevision: 9, expectedEpoch: id(3), edit: {type: "resume"}});
    expect(JSON.parse(f.saved.getItem("pending")!).resume.request).toEqual(value);
    state = {...state, revision: 10, paused: false}; return state;
  });
  const queueOwner = new AgentInputQueueController({read, edit}, {conversationId: id(1)}, () => id(7));
  const port = independentQueueResume(() => ({controller: queueOwner, release: () => {}}), read);
  await owner.continuePaused(port);
  expect(owner.getSnapshot().phase).toBe("started"); expect(edit).toHaveBeenCalledOnce();
  expect(f.start).toHaveBeenLastCalledWith(startRequest, id(3)); expect(f.start).toHaveBeenCalledTimes(2);
});
