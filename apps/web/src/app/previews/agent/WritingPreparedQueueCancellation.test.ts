import {afterEach, expect, it, vi} from "vitest";
import type {AgentWritingQueueCancelRequest} from "@research-assistant/core/agent";
import {acquireWritingSubmissionController, releaseWritingSubmissionController} from "@research-assistant/workbench-sdk/agent-writing";
import {WritingSubmissionController, forgetWritingSubmissionController} from "../../../core/features/agent/writingSubmissionController";
import {registerStoragePersistence} from "../../../core/runtime/storagePersistence";
import {selectAppStorage} from "../../../core/runtime/appStorage";
import {WritingDiffDemoStorage} from "./WritingDiffDemoStorage";
import {WritingDiffDemoAgentStorage} from "./WritingDiffDemoAgentStorage";
import {createWritingDiffDemoRuntime} from "./writingDiffDemoRuntime";

class MemoryStorage implements Storage {
  readonly values = new Map<string, string>();
  get length() {return this.values.size;}
  key(index: number) {return [...this.values.keys()][index] ?? null;}
  getItem(key: string) {return this.values.get(key) ?? null;}
  setItem(key: string, value: string) {this.values.set(key, value);}
  removeItem(key: string) {this.values.delete(key);}
  clear() {this.values.clear();}
}
const cleanups: (() => void)[] = [];
afterEach(() => {for (const cleanup of cleanups.splice(0)) cleanup(); vi.restoreAllMocks();});
async function fixture() {
  const storage = new MemoryStorage(); selectAppStorage(storage);
  const scope = {manuscriptId: crypto.randomUUID(), conversationId: crypto.randomUUID()}, inputId = crypto.randomUUID();
  const files = new WritingDiffDemoStorage(scope.manuscriptId, []), agent = new WritingDiffDemoAgentStorage(files, undefined);
  const runtime = createWritingDiffDemoRuntime(files, agent, scope.manuscriptId, scope.conversationId);
  const queue = await runtime.queue.gateway.edit({...scope, requestId: crypto.randomUUID(), expectedRevision: 0,
    edit: {type: "add", inputId, text: "Externally prepared input", userInput: []}});
  // Preparation belongs to the external host. Core receives its exact cancel
  // intent without adopting the host's preview as an ordinary submission.
  const prepared = await runtime.queue.preview.prepare({inputId, expectedRevision: queue.revision, submission: {
    conversationId: scope.conversationId, requestId: crypto.randomUUID(), text: "Externally prepared input", userInput: [],
    context: {manuscriptId: scope.manuscriptId, files: []}}});
  const intent: AgentWritingQueueCancelRequest = {...scope, inputId, requestId: crypto.randomUUID(),
    previewRequestId: prepared.preview.requestId, expectedRevision: prepared.queue.revision};
  const controllers: WritingSubmissionController[] = [];
  const open = (disabled = false) => {
    const controller = new WritingSubmissionController(runtime.gateways.submission, runtime.gateways.turns,
      runtime.queue.preview, {scope, storage}, () => disabled);
    controllers.push(controller); return controller;
  };
  const cancel = vi.spyOn(runtime.queue.preview, "cancel"), status = vi.spyOn(runtime.queue.preview, "cancelStatus");
  const recover = vi.spyOn(runtime.queue.preview, "recover"), prepare = vi.spyOn(runtime.queue.preview, "prepare");
  const start = vi.spyOn(runtime.queue.preview, "start"), ordinaryStart = vi.spyOn(runtime.gateways.submission, "startWriting");
  const readCurrent = vi.fn(async () => ({...prepared, queue: await runtime.queue.gateway.read(scope)}));
  cleanups.push(() => {controllers.forEach(value => value.dispose()); runtime.dispose();
    expect(prepare).not.toHaveBeenCalled(); expect(start).not.toHaveBeenCalled(); expect(ordinaryStart).not.toHaveBeenCalled();});
  return {storage, scope, intent, runtime, open, cancel, status, recover, readCurrent, prepared};
}
function persistence(storage: Storage, flush: () => Promise<void>, retry = async () => {}, error: () => Error | undefined = () => undefined) {
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, get error() {return error();}, resetting: false, flush, retry,
    reset: vi.fn(), subscribe: vi.fn(() => () => {}), close: vi.fn()});
}

it("flushes one full frozen intent before dispatch, including cleanup after the instance stops accepting new work", async () => {
  const f = await fixture(), controller = f.open(true);
  let release!: () => void;
  const flush = vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => {release = resolve;})).mockResolvedValue(undefined);
  persistence(f.storage, flush);
  const supplied = {...f.intent}, pending = controller.beginPreparedQueueCancellation(supplied);
  supplied.expectedRevision += 100;
  expect(f.cancel).not.toHaveBeenCalled(); expect(controller.isQueuedCancellationCommitted(f.intent)).toBe(false);
  expect(f.storage.length).toBe(1);
  expect(JSON.parse(f.storage.getItem(f.storage.key(0)!)!).intent).toEqual(f.intent);
  release(); await pending;
  expect(f.cancel).toHaveBeenCalledExactlyOnceWith(f.intent);
  expect(f.recover).not.toHaveBeenCalled(); expect(f.status).not.toHaveBeenCalled();
  expect(controller.isQueuedCancellationCommitted(f.intent)).toBe(true);
  const reversed = Object.fromEntries(Object.entries(f.intent).reverse()) as unknown as AgentWritingQueueCancelRequest;
  expect(controller.isQueuedCancellationCommitted(reversed)).toBe(true);
  expect(controller.isQueuedCancellationCommitted({...f.intent, requestId: crypto.randomUUID()})).toBe(false);
  await controller.beginPreparedQueueCancellation(f.intent);
  expect(f.cancel).toHaveBeenCalledTimes(1); expect(f.storage.length).toBe(0);
});

it.each(["setItem", "flush"] as const)("retains the original intent after initial %s failure and dispatches only after retry", async failure => {
  const f = await fixture(), controller = f.open(); let failing = true;
  const write = f.storage.setItem.bind(f.storage);
  vi.spyOn(f.storage, "setItem").mockImplementation((key, value) => {if (failing && failure === "setItem") throw new Error("quota"); write(key, value);});
  persistence(f.storage, async () => {if (failing && failure === "flush") throw new Error("quota");});
  await controller.beginPreparedQueueCancellation(f.intent);
  expect(f.cancel).not.toHaveBeenCalled(); expect(controller.isQueuedCancellationCommitted(f.intent)).toBe(false);
  expect(controller.getSnapshot().cancellation?.intent).toEqual(f.intent);
  failing = false; await controller.retryQueuedCancellation();
  expect(f.cancel).toHaveBeenCalledExactlyOnceWith(f.intent);
  expect(controller.isQueuedCancellationCommitted(f.intent)).toBe(true);
});

it("requires reliable local cleanup even after Native committed and retries cleanup without another RPC", async () => {
  const f = await fixture(), controller = f.open();
  const flush = vi.fn().mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("cleanup failed")).mockResolvedValue(undefined);
  persistence(f.storage, flush);
  await controller.beginPreparedQueueCancellation(f.intent);
  expect(controller.getSnapshot().cancellation).toMatchObject({phase: "localError", knownCommitted: true});
  expect(controller.isQueuedCancellationCommitted(f.intent)).toBe(false);
  await controller.checkQueuedCancellation();
  expect(controller.isQueuedCancellationCommitted(f.intent)).toBe(true);
  expect(f.cancel).toHaveBeenCalledTimes(1); expect(f.status).not.toHaveBeenCalled();
});

it("recovers lost ACK on cold open through the original proof without repeating cancellation", async () => {
  const f = await fixture(), controller = f.open(); f.runtime.queue.failNextCancel("afterCommit");
  await controller.beginPreparedQueueCancellation(f.intent);
  expect(controller.isQueuedCancellationCommitted(f.intent)).toBe(false);
  controller.dispose(); const reopened = f.open();
  await reopened.beginPreparedQueueCancellation(f.intent);
  expect(f.cancel).toHaveBeenCalledTimes(1); expect(f.status).not.toHaveBeenCalled();
  await reopened.checkQueuedCancellation();
  expect(f.status).toHaveBeenCalledExactlyOnceWith(f.intent);
  expect(reopened.isQueuedCancellationCommitted(f.intent)).toBe(true); expect(f.storage.length).toBe(0);
});

it("uses the exact-current reader only for explicit retry of a definitive rejection and tracks O2 completely", async () => {
  const f = await fixture(), controller = f.open();
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: f.intent.expectedRevision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "New tail"}});
  await controller.beginPreparedQueueCancellation(f.intent);
  expect(controller.getSnapshot().cancellation?.definitiveConflict).toBe(true);
  await controller.checkQueuedCancellation(); expect(f.readCurrent).not.toHaveBeenCalled();
  await controller.retryQueuedCancellation(f.readCurrent);
  expect(f.readCurrent).toHaveBeenCalledExactlyOnceWith(f.scope); expect(f.recover).not.toHaveBeenCalled();
  const replacement = f.cancel.mock.calls[1]![0];
  expect(replacement.requestId).not.toBe(f.intent.requestId);
  expect({...replacement, requestId: f.intent.requestId, expectedRevision: f.intent.expectedRevision}).toEqual(f.intent);
  expect(replacement.expectedRevision).toBe(f.intent.expectedRevision + 1);
  expect(controller.isQueuedCancellationCommitted(f.intent)).toBe(false);
  expect(controller.isQueuedCancellationCommitted(replacement)).toBe(true);
});

it("never rebases an unknown operation merely because a supplied reader can see a newer queue", async () => {
  const f = await fixture(), controller = f.open(); f.runtime.queue.failNextCancel("beforeCommit");
  await controller.beginPreparedQueueCancellation(f.intent);
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: f.intent.expectedRevision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "New tail"}});
  await controller.retryQueuedCancellation(f.readCurrent);
  expect(f.readCurrent).not.toHaveBeenCalled();
  expect(f.cancel.mock.calls.map(([intent]) => intent)).toEqual([f.intent, f.intent]);
  expect(controller.isQueuedCancellationCommitted(f.intent)).toBe(false);
});

it.each(["input", "request", "claim"] as const)("rejects a foreign %s from the explicit current reader without replacing the original intent", async field => {
  const f = await fixture(), controller = f.open();
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: f.intent.expectedRevision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "New tail"}});
  await controller.beginPreparedQueueCancellation(f.intent);
  const foreign = {...await f.readCurrent()}; f.readCurrent.mockClear();
  if (field === "input") foreign.inputId = crypto.randomUUID();
  else if (field === "request") foreign.preview = {...foreign.preview, requestId: crypto.randomUUID()};
  else foreign.queue = {...foreign.queue, preparingInputId: crypto.randomUUID()};
  f.readCurrent.mockResolvedValueOnce(foreign);
  await controller.retryQueuedCancellation(f.readCurrent);
  expect(f.cancel).toHaveBeenCalledTimes(1);
  expect(controller.getSnapshot().cancellation?.intent).toEqual(f.intent);
  expect(controller.isQueuedCancellationCommitted(f.intent)).toBe(false);
  expect(JSON.parse(f.storage.getItem(f.storage.key(0)!)!).intent).toEqual(f.intent);
});

it("keeps the same operation on cold reopen after disposal during the first reliable save", async () => {
  const f = await fixture(), controller = f.open(); let release!: () => void;
  persistence(f.storage, vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => {release = resolve;})).mockResolvedValue(undefined));
  const pending = controller.beginPreparedQueueCancellation(f.intent);
  controller.dispose(); release(); await pending;
  expect(f.cancel).not.toHaveBeenCalled(); expect(f.storage.length).toBe(1);
  const reopened = f.open(); await reopened.checkQueuedCancellation();
  expect(f.cancel).not.toHaveBeenCalled(); expect(reopened.isQueuedCancellationCommitted(f.intent)).toBe(false);
  await reopened.retryQueuedCancellation();
  expect(f.cancel).toHaveBeenCalledExactlyOnceWith(f.intent);
  expect(reopened.isQueuedCancellationCommitted(f.intent)).toBe(true);
});

it("ignores a disposed owner's late current-reader result after a new owner completed O2", async () => {
  const f = await fixture(), controller = f.open();
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: f.intent.expectedRevision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "New tail"}});
  await controller.beginPreparedQueueCancellation(f.intent);
  const current = await f.readCurrent(); f.readCurrent.mockClear();
  let release!: () => void;
  const gate = new Promise<void>(resolve => {release = resolve;});
  const heldReader = vi.fn(async () => {await gate; return current;});
  const pending = controller.retryQueuedCancellation(heldReader);
  await vi.waitFor(() => expect(heldReader).toHaveBeenCalledOnce());
  controller.dispose(); const reopened = f.open();
  await reopened.retryQueuedCancellation(f.readCurrent);
  const replacement = f.cancel.mock.calls[1]![0];
  expect(reopened.isQueuedCancellationCommitted(replacement)).toBe(true);
  release(); await pending;
  expect(f.cancel).toHaveBeenCalledTimes(2); expect(f.storage.length).toBe(0);
  expect(controller.isQueuedCancellationCommitted(replacement)).toBe(false);
  expect(reopened.isQueuedCancellationCommitted(replacement)).toBe(true);
});

it("preserves an ordinary queued owner and rejects foreign scope without invoking cancellation", async () => {
  const f = await fixture(), controller = f.open();
  await controller.beginPreparedQueueCancellation({...f.intent, conversationId: crypto.randomUUID()});
  await controller.beginPreparedQueueCancellation({...f.intent, manuscriptId: crypto.randomUUID()});
  expect(controller.getSnapshot().phase).toBe("idle"); expect(f.storage.length).toBe(0);
  await controller.recoverQueued(f.scope); const original = controller.getSnapshot();
  await controller.beginPreparedQueueCancellation(f.intent);
  expect(controller.getSnapshot()).toBe(original); expect(f.cancel).not.toHaveBeenCalled();
});

it("keeps a foreign cancellation intact and does not adopt its result for this intent", async () => {
  const f = await fixture(), controller = f.open(); f.runtime.queue.failNextCancel("beforeCommit");
  await controller.beginPreparedQueueCancellation(f.intent);
  const saved = [...f.storage.values.values()];
  await controller.beginPreparedQueueCancellation({...f.intent, requestId: crypto.randomUUID()});
  expect([...f.storage.values.values()]).toEqual(saved); expect(f.cancel).toHaveBeenCalledTimes(1);
  expect(controller.isQueuedCancellationCommitted(f.intent)).toBe(false);
});

it("retains one pooled Core owner across views while its cancellation is awaiting reliable save", async () => {
  const f = await fixture(), key = crypto.randomUUID();
  const first = acquireWritingSubmissionController(key, f.runtime.gateways.submission, f.runtime.gateways.turns,
    f.runtime.queue.preview, {scope: f.scope, storage: f.storage});
  cleanups.push(() => forgetWritingSubmissionController(key, first));
  const second = acquireWritingSubmissionController(key, f.runtime.gateways.submission, f.runtime.gateways.turns,
    f.runtime.queue.preview, {scope: f.scope, storage: f.storage});
  expect(first).toBe(second);
  let release!: () => void;
  persistence(f.storage, vi.fn().mockImplementationOnce(() => new Promise<void>(resolve => {release = resolve;})).mockResolvedValue(undefined));
  const pending = first.beginPreparedQueueCancellation(f.intent);
  releaseWritingSubmissionController(key, first); releaseWritingSubmissionController(key, second);
  const reopened = acquireWritingSubmissionController(key, f.runtime.gateways.submission, f.runtime.gateways.turns,
    f.runtime.queue.preview, {scope: f.scope, storage: f.storage});
  expect(reopened).toBe(first);
  release(); await pending;
  expect(reopened.isQueuedCancellationCommitted(f.intent)).toBe(true);
  expect(f.cancel).toHaveBeenCalledExactlyOnceWith(f.intent);
  releaseWritingSubmissionController(key, reopened);
});
