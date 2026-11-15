import { afterEach, expect, it, vi } from "vitest";
import { WritingSubmissionController } from "./writingSubmissionController";
import { WritingDiffDemoStorage } from "../../../app/previews/agent/WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "../../../app/previews/agent/WritingDiffDemoAgentStorage";
import { createWritingDiffDemoRuntime } from "../../../app/previews/agent/writingDiffDemoRuntime";
import { registerStoragePersistence } from "../../runtime/storagePersistence";
import { selectAppStorage } from "../../runtime/appStorage";
import { writingQueueCancellationMessage } from "./writingQueueCancellationMessage";

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
    edit: {type: "add", inputId, text: "original input", userInput: []}});
  const prepared = await runtime.queue.preview.prepare({inputId, expectedRevision: queue.revision, submission: {
    conversationId: scope.conversationId, requestId: crypto.randomUUID(), text: "original input", userInput: [], context: {manuscriptId: scope.manuscriptId, files: []}}});
  const controllers: WritingSubmissionController[] = [];
  const reopen = () => {const controller = new WritingSubmissionController(runtime.gateways.submission, runtime.gateways.turns,
    runtime.queue.preview, {scope, storage}); controllers.push(controller); return controller;};
  const controller = reopen(); await controller.recoverQueued(scope);
  const cancelOriginal = runtime.queue.preview.cancel, cancel = vi.spyOn(runtime.queue.preview, "cancel"), status = vi.spyOn(runtime.queue.preview, "cancelStatus");
  cleanups.push(() => {controllers.forEach(value => value.dispose()); runtime.dispose();});
  return {scope, inputId, storage, runtime, prepared, controller, reopen, cancel, status, cancelOriginal};
}
function persistence(storage: Storage, flush: () => Promise<void>, retry = async () => {}, error: () => Error | undefined = () => undefined) {
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, get error() {return error();}, resetting: false, flush, retry,
    reset: vi.fn(), subscribe: vi.fn(() => () => {}), close: vi.fn()});
}

it("flushes the complete cancellation before native dispatch and retries the same operation after local failure", async () => {
  const f = await fixture(); let failing = true;
  const retry = vi.fn(async () => {failing = false;});
  persistence(f.storage, async () => {if (failing) throw new Error("quota");}, retry, () => failing ? new Error("quota") : undefined);
  const operation = crypto.randomUUID(); await f.controller.cancelQueued(operation);
  expect(f.cancel).not.toHaveBeenCalled(); expect(f.status).not.toHaveBeenCalled();
  const intent = f.controller.getSnapshot().cancellation?.intent;
  expect(intent).toEqual({...f.scope, requestId: operation, inputId: f.inputId, previewRequestId: f.prepared.preview.requestId, expectedRevision: f.prepared.queue.revision});
  expect(JSON.parse([...f.storage.values.values()][0]!).intent).toEqual(intent);
  await f.controller.retryQueuedCancellation(); expect(retry).toHaveBeenCalledOnce();
  expect(f.cancel).toHaveBeenCalledExactlyOnceWith(intent); expect(f.controller.getSnapshot()).toMatchObject({phase: "idle", queue: null});
});

it("reopens an unknown committed cancellation without its preview pointer and only inspects the original proof", async () => {
  const f = await fixture(); f.runtime.queue.failNextCancel("afterCommit");
  await f.controller.cancelQueued(crypto.randomUUID()); const intent = f.controller.getSnapshot().cancellation!.intent!;
  const current = await f.runtime.queue.gateway.read(f.scope); expect(current.preparingInputId).toBeNull();
  const newer = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: current.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "newer tail"}});
  f.controller.dispose(); const reopened = f.reopen();
  expect(reopened.getSnapshot()).toMatchObject({phase: "error", error: "cancel", preview: null, cancellation: {intent}});
  expect(f.cancel).toHaveBeenCalledTimes(1); expect(f.status).not.toHaveBeenCalled();
  await reopened.checkQueuedCancellation();
  expect(f.status).toHaveBeenCalledExactlyOnceWith(intent); expect(f.cancel).toHaveBeenCalledTimes(1);
  expect(reopened.getSnapshot()).toMatchObject({phase: "idle", queue: null}); expect(f.storage.length).toBe(0);
  expect(await f.runtime.queue.gateway.read(f.scope)).toEqual(newer);
});

it("checks notFound on reopen and only an explicit retry reuses the original cancellation", async () => {
  const f = await fixture(); f.runtime.queue.failNextCancel("beforeCommit");
  await f.controller.cancelQueued(crypto.randomUUID()); const intent = f.controller.getSnapshot().cancellation!.intent!;
  f.controller.dispose(); const reopened = f.reopen(); await reopened.checkQueuedCancellation();
  expect(reopened.getSnapshot().cancellation?.phase).toBe("notFound"); expect(f.cancel).toHaveBeenCalledTimes(1);
  await reopened.retryQueuedCancellation(); expect(f.cancel.mock.calls.map(([request]) => request)).toEqual([intent, intent]);
  expect(reopened.getSnapshot().phase).toBe("idle");
});

it("retains an unconfirmed operation and safe diagnostics without replaying mutation or starting", async () => {
  const f = await fixture(); f.runtime.queue.failNextCancel("unavailable");
  const start = vi.spyOn(f.runtime.queue.preview, "start"); await f.controller.cancelQueued(crypto.randomUUID());
  const intent = f.controller.getSnapshot().cancellation!.intent!;
  await f.controller.checkQueuedCancellation(); expect(f.cancel).toHaveBeenCalledTimes(1); expect(start).not.toHaveBeenCalled();
  expect(f.controller.getSnapshot().cancellation).toMatchObject({phase: "unconfirmed", intent});
  const message = writingQueueCancellationMessage(f.controller.getSnapshot().cancellation);
  expect(message).toContain(`operation=${intent.requestId}`); expect(message).toContain("mock_owner_unavailable");
  await f.controller.retryQueuedCancellation(); expect(f.cancel.mock.calls.map(([request]) => request)).toEqual([intent, intent]);
});

it("retries cleanup only after an acknowledged cancellation even if the local remove flush failed", async () => {
  const f = await fixture(), flush = vi.fn().mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("cleanup failed")).mockResolvedValue(undefined);
  persistence(f.storage, flush); await f.controller.cancelQueued(crypto.randomUUID());
  expect(f.controller.getSnapshot().cancellation).toMatchObject({phase: "localError", knownCommitted: true});
  await f.controller.checkQueuedCancellation(); expect(f.cancel).toHaveBeenCalledTimes(1); expect(f.status).not.toHaveBeenCalled();
  expect(f.controller.getSnapshot().phase).toBe("idle");
});

it("preserves the locator after a late native result reaches a disposed owner", async () => {
  const f = await fixture(), original = f.cancelOriginal;
  let release!: () => void; const gate = new Promise<void>(resolve => {release = resolve;});
  f.cancel.mockImplementation(async request => {await gate; return original(request);});
  const canceling = f.controller.cancelQueued(crypto.randomUUID());
  await vi.waitFor(() => expect(f.cancel).toHaveBeenCalledOnce());
  f.controller.dispose(); release(); await canceling;
  expect(f.storage.length).toBe(1); expect(f.controller.getSnapshot().phase).toBe("canceling");
  const reopened = f.reopen(); await reopened.checkQueuedCancellation();
  expect(f.cancel).toHaveBeenCalledTimes(1); expect(reopened.getSnapshot().cancellation).toMatchObject({phase: "ready", knownCommitted: true, cause: null});
});

it("does not dispatch cancellation after its owner is disposed while the first flush is pending", async () => {
  const f = await fixture(); let release!: () => void;
  persistence(f.storage, () => new Promise<void>(resolve => {release = resolve;}));
  const pending = f.controller.cancelQueued(crypto.randomUUID());
  expect(f.cancel).not.toHaveBeenCalled(); f.controller.dispose(); release(); await pending;
  expect(f.cancel).not.toHaveBeenCalled(); expect(f.storage.length).toBe(1);
});

it("does not let a disposed owner's late rejection erase the current owner's persisted rejection evidence", async () => {
  const f = await fixture(), queue = await f.runtime.queue.gateway.read(f.scope);
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: queue.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "concurrent tail"}});
  let release!: () => void; const gate = new Promise<void>(resolve => {release = resolve;});
  f.cancel.mockImplementationOnce(async request => {await gate; return f.cancelOriginal(request);});
  const pending = f.controller.cancelQueued(crypto.randomUUID());
  await vi.waitFor(() => expect(f.cancel).toHaveBeenCalledOnce());
  f.controller.dispose(); const current = f.reopen(); await current.retryQueuedCancellation();
  expect(current.getSnapshot().cancellation?.definitiveConflict).toBe(true);
  const saved = [...f.storage.values.values()][0]!;
  expect(JSON.parse(saved).definitiveConflict).toBe(true);
  release(); await pending;
  expect([...f.storage.values.values()][0]).toBe(saved);
  current.dispose(); expect(f.reopen().getSnapshot().cancellation?.definitiveConflict).toBe(true);
});

it("discloses a safe definitive domain rejection while retaining the exact cancellation intent", async () => {
  const f = await fixture(), current = await f.runtime.queue.gateway.read(f.scope);
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: current.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "concurrent tail"}});
  await f.controller.cancelQueued(crypto.randomUUID()); const state = f.controller.getSnapshot().cancellation;
  const message = writingQueueCancellationMessage(state);
  expect(message).toContain("DATA_SQL_DOMAIN"); expect(message).toContain("reason=conversation changed since it was read"); expect(message).toContain("被拒绝");
  expect(state?.intent?.expectedRevision).toBe(f.prepared.queue.revision); expect(f.storage.length).toBe(1);
});

it("persists a definitive stale-Q rejection across reopen and only explicit retry cancels the same preview with O2 and current Q", async () => {
  const f = await fixture(), before = await f.runtime.queue.gateway.read(f.scope);
  const current = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: before.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "tail"}});
  await f.controller.cancelQueued(crypto.randomUUID()); const original = f.controller.getSnapshot().cancellation!.intent!;
  expect(f.controller.getSnapshot().cancellation?.definitiveConflict).toBe(true);
  f.controller.dispose(); const reopened = f.reopen();
  expect(reopened.getSnapshot().cancellation?.definitiveConflict).toBe(true);
  await reopened.checkQueuedCancellation(); expect(f.cancel).toHaveBeenCalledOnce();
  await reopened.retryQueuedCancellation(); const replacement = f.cancel.mock.calls[1]![0];
  expect(replacement.requestId).not.toBe(original.requestId);
  expect({...replacement, requestId: original.requestId, expectedRevision: original.expectedRevision}).toEqual(original);
  expect(replacement.expectedRevision).toBe(current.revision); expect(reopened.getSnapshot().phase).toBe("idle");
  const after = await f.runtime.queue.gateway.read(f.scope); expect(after.inputs).toEqual(current.inputs); expect(after.paused).toBe(true);
});

it("does not rebase an Unknown operation on notFound even when a newer Q is visible", async () => {
  const f = await fixture(); f.runtime.queue.failNextCancel("beforeCommit");
  await f.controller.cancelQueued(crypto.randomUUID()); const original = f.controller.getSnapshot().cancellation!.intent!;
  const queue = await f.runtime.queue.gateway.read(f.scope);
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: queue.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "tail"}});
  await f.controller.retryQueuedCancellation();
  expect(f.cancel.mock.calls.map(([request]) => request)).toEqual([original, original]);
  expect(f.controller.getSnapshot().cancellation?.intent).toEqual(original);
});

it("does not read a preview or dispatch after disposal while flushing a definitive rejection for refresh", async () => {
  const f = await fixture(), before = await f.runtime.queue.gateway.read(f.scope);
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: before.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "tail"}});
  await f.controller.cancelQueued(crypto.randomUUID());
  let release!: () => void; const flush = vi.fn(() => new Promise<void>(resolve => {release = resolve;}));
  persistence(f.storage, flush); const recover = vi.spyOn(f.runtime.queue.preview, "recover");
  const retry = f.controller.retryQueuedCancellation(); await vi.waitFor(() => expect(flush).toHaveBeenCalledOnce());
  f.controller.dispose(); release(); await retry;
  expect(recover).not.toHaveBeenCalled(); expect(f.cancel).toHaveBeenCalledOnce();
});

it("retains O2 after its acknowledged cleanup fails without allocating or sending another operation", async () => {
  const f = await fixture(), before = await f.runtime.queue.gateway.read(f.scope);
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: before.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "tail"}});
  const flush = vi.fn().mockResolvedValueOnce(undefined).mockResolvedValueOnce(undefined).mockResolvedValueOnce(undefined)
    .mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("cleanup failed")).mockResolvedValue(undefined);
  persistence(f.storage, flush); await f.controller.cancelQueued(crypto.randomUUID());
  await f.controller.retryQueuedCancellation();
  const replacement = f.cancel.mock.calls[1]![0], checked = f.status.mock.calls.length;
  expect(f.controller.getSnapshot().cancellation).toMatchObject({knownCommitted: true, intent: replacement, phase: "localError"});
  await f.controller.retryQueuedCancellation(); expect(f.cancel).toHaveBeenCalledTimes(2); expect(f.status).toHaveBeenCalledTimes(checked);
  expect(f.controller.getSnapshot().phase).toBe("idle");
});

it.each(["setItem", "flush"] as const)("retains the newly allocated O2 after its first %s fails and dispatches only after retry saves it", async failure => {
  const f = await fixture(), before = await f.runtime.queue.gateway.read(f.scope);
  await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: before.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "tail"}});
  let writes = 0, flushes = 0; const write = f.storage.setItem.bind(f.storage);
  vi.spyOn(f.storage, "setItem").mockImplementation((key, value) => {if (++writes === 4 && failure === "setItem") throw new Error("quota"); write(key, value);});
  persistence(f.storage, async () => {if (++flushes === 4 && failure === "flush") throw new Error("quota");});
  await f.controller.cancelQueued(crypto.randomUUID()); const first = f.controller.getSnapshot().cancellation!.intent!;
  await f.controller.retryQueuedCancellation(); const replacement = f.controller.getSnapshot().cancellation!.intent!;
  expect(replacement.requestId).not.toBe(first.requestId); expect(f.cancel).toHaveBeenCalledOnce();
  await f.controller.retryQueuedCancellation(); expect(f.cancel).toHaveBeenCalledTimes(2);
  expect(f.cancel.mock.calls[1]![0]).toEqual(replacement); expect(f.controller.getSnapshot().phase).toBe("idle");
});

it("does not replace a rejected cancellation when the current I belongs to a different preview R", async () => {
  const f = await fixture(), before = await f.runtime.queue.gateway.read(f.scope);
  const current = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: before.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "tail"}});
  await f.controller.cancelQueued(crypto.randomUUID()); const original = f.controller.getSnapshot().cancellation!.intent!;
  await f.runtime.queue.preview.cancel({...original, requestId: crypto.randomUUID(), expectedRevision: current.revision});
  const paused = await f.runtime.queue.gateway.read(f.scope), resumed = await f.runtime.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: paused.revision,
    edit: {type: "resumeInput", inputId: f.inputId}});
  await f.runtime.queue.preview.prepare({inputId: f.inputId, expectedRevision: resumed.revision, submission: {conversationId: f.scope.conversationId,
    requestId: crypto.randomUUID(), text: "original input", userInput: [], context: {manuscriptId: f.scope.manuscriptId, files: []}}});
  const calls = f.cancel.mock.calls.length; await f.controller.retryQueuedCancellation();
  expect(f.cancel).toHaveBeenCalledTimes(calls); expect(f.controller.getSnapshot().cancellation?.intent).toEqual(original);
  expect(writingQueueCancellationMessage(f.controller.getSnapshot().cancellation)).toContain("QUEUE_CANCEL_PREVIEW_CHANGED");
});

it.each(["AGENT_PREVIEW_SYNTHETIC_IO_ACK_LOST", "AGENT_PREVIEW_SYNTHETIC_IO_BEFORE_COMMIT", "AGENT_PREVIEW_SCENARIO_CANCELED"])("retains the safe preview I/O code %s with original identities", async code => {
  const f = await fixture(); f.cancel.mockRejectedValueOnce(code); await f.controller.cancelQueued(crypto.randomUUID());
  const state = f.controller.getSnapshot().cancellation!, message = writingQueueCancellationMessage(state);
  expect(message).toContain(code); for (const id of [state.intent!.requestId, f.inputId, f.scope.conversationId, f.prepared.preview.requestId]) expect(message).toContain(id);
});
