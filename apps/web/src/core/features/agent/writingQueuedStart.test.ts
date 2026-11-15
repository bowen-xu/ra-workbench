import {expect, it, vi} from "vitest";
import type {AgentSubmissionPreview, AgentWritingQueuedStartRequest, AgentWritingQueuedStartStatusResponse,
  AgentWritingQueueEditRequest, AgentWritingQueueResponse, AgentWritingStartResponse} from "@research-assistant/core/agent";
import type {AgentInputQueuePreviewGateway} from "@research-assistant/data/agent";
import {registerStoragePersistence} from "../../runtime/storagePersistence";
import {WritingQueuedStart} from "./writingQueuedStart";
import type {WritingQueuedStartQueuePort} from "./writingQueuedStartQueue";
import {writingQueuedStartMessage} from "./writingQueuedStartMessage";

const c = "11111111-1111-4111-8111-111111111111", m = "22222222-2222-4222-8222-222222222222";
const r = "33333333-3333-4333-8333-333333333333", i = "44444444-4444-4444-8444-444444444444";
const t = "55555555-5555-4555-8555-555555555555", s = "66666666-6666-4666-8666-666666666666";
const scope = {conversationId: c, manuscriptId: m};
const intent: AgentWritingQueuedStartRequest = {inputId: i, expectedRevision: 5, start: {...scope, requestId: r,
  contextSnapshotId: s, model: {providerId: "deepseek", modelId: "original-model", revision: "original-revision"}, mode: "deepSeekNonThinking"}};
const preview: AgentSubmissionPreview = {schemaVersion: 1, ...scope, requestId: r, contextSnapshotId: s, turnId: t,
  input: {text: "original input", model: intent.start.model, reasoningEffort: null, context: []}};
const result: AgentWritingStartResponse = {schemaVersion: 1, conversationId: c, requestId: r, turnId: t, alreadyStarted: false, outcome: null};
const preparedQueue: AgentWritingQueueResponse = {schemaVersion: 1, ...scope, revision: 5, paused: true, preparingInputId: i, inputs: [{inputId: i, text: "original input"}]};
const resume: AgentWritingQueueEditRequest = {...scope, requestId: "77777777-7777-4777-8777-777777777777", expectedRevision: 8, edit: {type: "resume"}};
function fixture() {
  const rows = new Map<string, string>();
  const storage: Storage = {get length() {return rows.size;}, key: index => [...rows.keys()][index] ?? null,
    getItem: key => rows.get(key) ?? null, setItem: (key, value) => {rows.set(key, value);}, removeItem: key => {rows.delete(key);}, clear: () => rows.clear()};
  const start = vi.fn<AgentInputQueuePreviewGateway["start"]>().mockRejectedValue(new Error("lost reply"));
  const startStatus = vi.fn<AgentInputQueuePreviewGateway["startStatus"]>().mockImplementation(async request => ({schemaVersion: 1, intent: request, status: {type: "prepared", preview}}));
  const unused = async (): Promise<never> => {throw new Error("unexpected preparation or cancellation");};
  const gateway: AgentInputQueuePreviewGateway = {start, startStatus, prepare: unused, prepareStatus: unused, recover: unused, cancel: unused, cancelStatus: unused};
  const changed = vi.fn(), observed = vi.fn(), cleanup = vi.fn(async (_original: AgentWritingQueuedStartRequest) => {});
  const open = (isAdmissionDisabled?: () => boolean) => new WritingQueuedStart(gateway, storage, changed, observed, cleanup, scope, isAdmissionDisabled);
  const read = vi.fn<WritingQueuedStartQueuePort["read"]>().mockResolvedValue(preparedQueue);
  const resumeQueue = vi.fn<WritingQueuedStartQueuePort["resume"]>().mockResolvedValue("acknowledged");
  return {storage, rows, start, startStatus, changed, observed, cleanup, open, queue: {read, resume: resumeQueue}, read, resumeQueue};
}
function persistent(storage: Storage, flush: () => Promise<void>): void {
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush, retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
}
function status(type: "prepared" | "admitted"): AgentWritingQueuedStartStatusResponse {
  return {schemaVersion: 1, intent, status: {type, preview}};
}

it("retains the proven queued start until context acceptance succeeds after reopening", async () => {
  const f=fixture(); f.start.mockResolvedValue(result); f.observed.mockRejectedValueOnce(new Error("context unconfirmed"));
  const owner=f.open(); await owner.begin(intent,t);
  expect(owner.getSnapshot()).toMatchObject({phase:"localError",result});
  expect(JSON.parse([...f.rows.values()][0]!).result).toEqual(result); expect(f.cleanup).not.toHaveBeenCalled();
  owner.dispose(); const reopened=f.open(); await reopened.retry(f.queue);
  expect(reopened.getSnapshot().phase).toBe("done"); expect(f.cleanup).toHaveBeenCalledOnce();
  expect(f.start).toHaveBeenCalledOnce(); expect(f.rows.size).toBe(0);
});

it("does not send the first full start until its reliable save finishes, and caller mutation cannot change it", async () => {
  const f = fixture(); let release!: () => void;
  persistent(f.storage, () => new Promise<void>(resolve => {release = resolve;}));
  const owner = f.open(), caller = structuredClone(intent), work = owner.begin(caller, t);
  expect(f.start).not.toHaveBeenCalled(); expect(owner.getSnapshot().phase).toBe("saving");
  (caller.start.model as {modelId: string}).modelId = "later model"; (caller as {expectedRevision: number}).expectedRevision = 99;
  release(); await work;
  expect(f.start).toHaveBeenCalledExactlyOnceWith(intent);
  expect(owner.getSnapshot()).toMatchObject({phase: "unknown", intent, priorUnknown: true});
  expect(JSON.parse([...f.rows.values()][0]!).intent).toEqual(intent);
});
it("a failed save preserves the actual intent locally and performs no start I/O", async () => {
  const f = fixture(); persistent(f.storage, async () => {throw new Error("durability unavailable");});
  const owner = f.open(); await owner.begin(intent, t);
  expect(f.start).not.toHaveBeenCalled(); expect(owner.getSnapshot()).toMatchObject({phase: "localError", intent});
  expect(f.observed).not.toHaveBeenCalled(); expect(f.cleanup).not.toHaveBeenCalled();
});
it("reopens Unknown read-only, then explicitly retries the exact first Q/model/mode after queue drift", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(intent, t); owner.dispose();
  const reopened = f.open(); expect(reopened.getSnapshot()).toMatchObject({phase: "unknown", intent, recovered: true});
  expect(f.start).toHaveBeenCalledOnce(); expect(f.startStatus).not.toHaveBeenCalled();
  await reopened.check(); expect(reopened.getSnapshot()).toMatchObject({phase: "prepared", intent});
  expect(f.read).not.toHaveBeenCalled(); expect(f.start).toHaveBeenCalledOnce();
  f.read.mockResolvedValue({...preparedQueue, revision: 19}); f.start.mockResolvedValue(result);
  await reopened.retry(f.queue);
  expect(f.start.mock.calls.map(([request]) => request)).toEqual([intent, intent]);
  expect(f.resumeQueue).not.toHaveBeenCalled(); expect(f.cleanup).toHaveBeenCalledExactlyOnceWith(intent);
  expect(reopened.getSnapshot().phase).toBe("done"); expect(f.rows.size).toBe(0);
});
it("a later definitive rejection cannot replace an earlier Unknown start revision", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(intent, t); owner.dispose();
  f.read.mockResolvedValue({...preparedQueue, revision: 19}); f.start.mockRejectedValue({code: "confirmationMismatch"});
  const reopened = f.open(); await reopened.retry(f.queue);
  expect(reopened.getSnapshot()).toMatchObject({phase: "unknown", intent, priorUnknown: true, knownRejected: false});
  expect(f.start.mock.calls.map(([request]) => request)).toEqual([intent, intent]);
  expect(JSON.parse([...f.rows.values()][0]!).intent).toEqual(intent);
});
it("only a known first rejection with the original prepared claim may reliably replace Q on explicit retry", async () => {
  const f = fixture(); f.start.mockRejectedValueOnce({code: "confirmationMismatch"}).mockResolvedValue(result);
  const owner = f.open(); await owner.begin(intent, t);
  expect(owner.getSnapshot()).toMatchObject({phase: "rejected", knownRejected: true, priorUnknown: false});
  f.read.mockResolvedValue({...preparedQueue, revision: 19}); await owner.retry(f.queue);
  expect(f.start.mock.calls.map(([request]) => request)).toEqual([intent, {...intent, expectedRevision: 19}]);
  expect(owner.getSnapshot().phase).toBe("done");
});
it("Started history recovers observation and cleanup without queue, workspace, preparation or another start", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(intent, t); owner.dispose();
  const completed = {...result, alreadyStarted: true, outcome: "completed" as const};
  f.startStatus.mockResolvedValue({schemaVersion: 1, intent, status: {type: "started", result: completed}});
  const reopened = f.open(); await reopened.check();
  expect(f.observed).toHaveBeenCalledExactlyOnceWith(intent, completed); expect(f.cleanup).toHaveBeenCalledExactlyOnceWith(intent);
  expect(f.start).toHaveBeenCalledOnce(); expect(f.read).not.toHaveBeenCalled(); expect(f.resumeQueue).not.toHaveBeenCalled();
  expect(reopened.getSnapshot()).toMatchObject({phase: "done", result: completed}); expect(f.rows.size).toBe(0);
});
it("a persisted Started result survives cleanup failure and reopening retries only local cleanup", async () => {
  const f = fixture(); f.start.mockResolvedValue(result); f.cleanup.mockRejectedValueOnce(new Error("related locator unavailable"));
  const owner = f.open(); await owner.begin(intent, t);
  expect(owner.getSnapshot()).toMatchObject({phase: "localError", result}); expect(f.observed).toHaveBeenCalledWith(intent, result);
  expect(JSON.parse([...f.rows.values()][0]!).result).toEqual(result); owner.dispose();
  const reopened = f.open(); await reopened.check();
  expect(f.start).toHaveBeenCalledOnce(); expect(f.startStatus).not.toHaveBeenCalled(); expect(f.read).not.toHaveBeenCalled();
  expect(f.cleanup.mock.calls).toEqual([[intent], [intent]]); expect(reopened.getSnapshot().phase).toBe("done");
});
it("a failed flush after removing the start locator retries that removal without repeating related cleanup", async () => {
  const f = fixture(); f.start.mockResolvedValue(result);
  let writes = 0; persistent(f.storage, async () => {if (++writes === 3) throw new Error("removal not flushed");});
  const owner = f.open(); await owner.begin(intent, t);
  expect(owner.getSnapshot()).toMatchObject({phase: "localError", result}); expect(f.rows.size).toBe(0);
  await owner.check();
  expect(owner.getSnapshot().phase).toBe("done"); expect(f.start).toHaveBeenCalledOnce(); expect(f.startStatus).not.toHaveBeenCalled();
  expect(f.cleanup).toHaveBeenCalledOnce(); expect(writes).toBe(4);
});
it("persists an Unknown Resume with the original start, replays that operation after reopening, and retains a later Pause", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(intent, t);
  f.startStatus.mockResolvedValue(status("admitted"));
  const laterPause = {...preparedQueue, revision: 12, preparingInputId: null, inputs: [], paused: true}; f.read.mockResolvedValue(laterPause);
  f.resumeQueue.mockImplementationOnce(async (_intent, original, beforeSend) => {
    expect(original).toBeNull(); await beforeSend(resume); throw new Error("Resume ACK lost");
  });
  await owner.retry(f.queue, true); expect(owner.getSnapshot()).toMatchObject({phase: "localError", intent, resumeCommand: resume});
  expect(JSON.parse([...f.rows.values()][0]!).resumeCommand).toEqual(resume); owner.dispose();
  const reopened = f.open(); expect(reopened.getSnapshot().resumeCommand).toEqual(resume);
  f.resumeQueue.mockImplementationOnce(async (originalStart, originalResume, beforeSend, eligible) => {
    expect(originalStart).toEqual(intent); expect(originalResume).toEqual(resume); expect(eligible()).toBe(true);
    await beforeSend(originalResume!); return "acknowledged";
  });
  await reopened.retry(f.queue, true);
  expect(f.start).toHaveBeenCalledOnce(); expect(f.resumeQueue).toHaveBeenCalledTimes(2);
  expect(reopened.getSnapshot()).toMatchObject({phase: "admitted", intent, resumeCommand: null, queuePaused: true});
  expect(JSON.parse([...f.rows.values()][0]!).intent).toEqual(intent);
});
it("retires only a freshly known-rejected Resume and waits for another explicit action before issuing a new Resume", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(intent, t); f.startStatus.mockResolvedValue(status("admitted"));
  const paused = {...preparedQueue, revision: 8, preparingInputId: null, inputs: []}; f.read.mockResolvedValue(paused);
  f.resumeQueue.mockImplementationOnce(async (_start, original, beforeSend) => {
    expect(original).toBeNull(); await beforeSend(resume); return "knownRejected";
  });
  await owner.retry(f.queue, true);
  expect(owner.getSnapshot()).toMatchObject({phase: "admitted", intent, resumeCommand: null, resumeAcknowledged: false});
  expect(f.resumeQueue).toHaveBeenCalledOnce(); expect(f.start).toHaveBeenCalledOnce();
  expect(JSON.parse([...f.rows.values()][0]!).resumeCommand).toBeNull();
  const nextResume: AgentWritingQueueEditRequest = {...resume, requestId: "88888888-8888-4888-8888-888888888888", expectedRevision: 14};
  f.read.mockResolvedValueOnce({...paused, revision: 14}).mockResolvedValueOnce({...paused, revision: 15, paused: false});
  f.resumeQueue.mockImplementationOnce(async (originalStart, originalResume, beforeSend) => {
    expect(originalStart).toEqual(intent); expect(originalResume).toBeNull(); await beforeSend(nextResume); return "acknowledged";
  });
  f.start.mockResolvedValue(result); await owner.retry(f.queue, true);
  expect(f.resumeQueue).toHaveBeenCalledTimes(2); expect(f.start.mock.calls.map(([request]) => request)).toEqual([intent, intent]);
  expect(owner.getSnapshot().phase).toBe("done");
});
it("an Unknown Resume remains the same saved operation when a later retry is rejected", async () => {
  const f = fixture(), owner = f.open(); await owner.begin(intent, t); f.startStatus.mockResolvedValue(status("admitted"));
  f.read.mockResolvedValue({...preparedQueue, revision: 12, preparingInputId: null, inputs: []});
  f.resumeQueue.mockImplementationOnce(async (_start, _original, beforeSend) => {await beforeSend(resume); throw new Error("lost Resume ACK");});
  await owner.retry(f.queue, true); owner.dispose(); const reopened = f.open();
  f.resumeQueue.mockImplementationOnce(async (_start, original, beforeSend) => {
    expect(original).toEqual(resume); await beforeSend(original!); return "unknown";
  });
  await reopened.retry(f.queue, true);
  expect(reopened.getSnapshot()).toMatchObject({phase: "admitted", intent, resumeCommand: resume, resumeAcknowledged: false});
  expect(JSON.parse([...f.rows.values()][0]!).resumeCommand).toEqual(resume); expect(f.start).toHaveBeenCalledOnce();
});

it("blocks the first queued start after its durable save and retires only that locator", async () => {
  const f = fixture(); let disabled = false, release!: () => void;
  const owner = f.open(() => disabled), gate = new Promise<void>(resolve => {release = resolve;});
  persistent(f.storage, () => gate); const work = owner.begin(intent, t);
  await vi.waitFor(() => expect(f.rows.size).toBe(1)); disabled = true; release(); await work;
  expect(f.start).not.toHaveBeenCalled(); expect(f.observed).not.toHaveBeenCalled(); expect(f.cleanup).not.toHaveBeenCalled();
  expect(owner.getSnapshot()).toMatchObject({phase: "localError", priorUnknown: false, knownRejected: false, cause: {code: "WRITING_DISPATCH_ADMISSION_CLOSED"}});
  expect(writingQueuedStartMessage(owner.getSnapshot())).toContain("此步骤尚未派发");
  expect(await owner.retireVerifiedUnstarted()).toBe(true); expect(f.rows.size).toBe(0);
  expect(f.cleanup).not.toHaveBeenCalled(); expect(f.observed).not.toHaveBeenCalled();
});

it("retains exact Unknown queued start replay while a new revision is blocked after queue read", async () => {
  const f = fixture(); let disabled = false; const owner = f.open(() => disabled);
  await owner.begin(intent, t); disabled = true; await owner.retry(f.queue);
  expect(f.start.mock.calls.map(([request]) => request)).toEqual([intent, intent]);
  const g = fixture(); disabled = false; g.start.mockRejectedValueOnce({code: "confirmationMismatch"});
  const other = g.open(() => disabled); await other.begin(intent, t);
  g.read.mockImplementationOnce(async () => {disabled = true; return {...preparedQueue, revision: 19};});
  await other.retry(g.queue);
  expect(g.start).toHaveBeenCalledOnce(); expect(other.getSnapshot().intent).toEqual(intent);
  expect(other.getSnapshot().cause).toMatchObject({code: "WRITING_DISPATCH_ADMISSION_CLOSED", step: "rebase"});
});

it("does not create a fresh Resume after its status and queue reads close the instance", async () => {
  const f = fixture(); let disabled = false; const owner = f.open(() => disabled);
  await owner.begin(intent, t); f.startStatus.mockResolvedValue(status("admitted"));
  f.read.mockImplementationOnce(async () => {disabled = true; return {...preparedQueue, preparingInputId: null};});
  await owner.retry(f.queue, true);
  expect(f.resumeQueue).not.toHaveBeenCalled(); expect(f.start).toHaveBeenCalledOnce();
  expect(owner.getSnapshot().cause).toMatchObject({code: "WRITING_DISPATCH_ADMISSION_CLOSED", step: "resume"});
});

it("does not grant cold or foreign queued start locators warm retirement authority", async () => {
  const f = fixture(), owner = f.open(() => true); await owner.begin(intent, t);
  owner.dispose(); const cold = f.open(() => true);
  expect(await cold.retireVerifiedUnstarted()).toBe(false); expect(f.rows.size).toBe(1);
  expect(cold.getSnapshot()).toMatchObject({recovered: true, priorUnknown: true});
  const g = fixture(), other = g.open(() => true); await other.begin(intent, t);
  const key = [...g.rows.keys()][0]!, foreign = JSON.stringify({...JSON.parse(g.rows.get(key)!), intent: {...intent, start: {...intent.start, requestId: s}}});
  g.rows.set(key, foreign);
  expect(await other.retireVerifiedUnstarted()).toBe(false); expect(g.rows.get(key)).toBe(foreign);
  expect(g.start).not.toHaveBeenCalled();
});

it("keeps failed unissued-start locator removal pending until its original flush is retried", async () => {
  const f = fixture(), owner = f.open(() => true); await owner.begin(intent, t);
  let fail = true; persistent(f.storage, async () => {if (f.rows.size === 0 && fail) throw new Error("retirement flush failed");});
  expect(await owner.retireVerifiedUnstarted()).toBe(false); expect(owner.unresolved()).toBe(true);
  expect(f.cleanup).not.toHaveBeenCalled(); expect(f.observed).not.toHaveBeenCalled();
  fail = false; expect(await owner.retireVerifiedUnstarted()).toBe(true); expect(owner.unresolved()).toBe(false);
  expect(f.start).not.toHaveBeenCalled();
});

it("requires fresh queued start admission after definitive rejection even without a new revision", async () => {
  const f = fixture(); let disabled = false; const owner = f.open(() => disabled);
  f.start.mockRejectedValueOnce({code: "confirmationMismatch"}); await owner.begin(intent, t);
  expect(owner.getSnapshot()).toMatchObject({knownRejected: true, priorUnknown: false});
  disabled = true; await owner.retry(f.queue);
  expect(f.start).toHaveBeenCalledOnce(); expect(owner.getSnapshot().intent).toEqual(intent);
  expect(owner.getSnapshot().cause).toMatchObject({code: "WRITING_DISPATCH_ADMISSION_CLOSED", step: "start"});
});

it("keeps a same-revision rejection retry fresh when admission closes during its save", async () => {
  const f = fixture(); let disabled = false, release!: () => void; const owner = f.open(() => disabled);
  f.start.mockRejectedValueOnce({code: "confirmationMismatch"}); await owner.begin(intent, t);
  const gate = new Promise<void>(resolve => {release = resolve;}); persistent(f.storage, () => gate);
  const work = owner.retry(f.queue); await vi.waitFor(() => expect(owner.getSnapshot().knownRejected).toBe(false));
  disabled = true; release(); await work;
  expect(f.start).toHaveBeenCalledOnce(); expect(owner.getSnapshot().intent).toEqual(intent);
  expect(owner.getSnapshot().cause).toMatchObject({code: "WRITING_DISPATCH_ADMISSION_CLOSED", step: "start"});
});
