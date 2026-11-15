import { expect, it, vi } from "vitest";
import type { AgentSubmissionPreview, AgentWritingPrepareRequest, AgentWritingStartRequest, AgentWritingStartResponse } from "@research-assistant/core/agent";
import { acquireWritingSubmissionController, forgetWritingSubmissionController, hasPendingWritingSubmissions, releaseWritingSubmissionController, resetWritingSubmissionControllersForTests, WritingSubmissionController } from "./writingSubmissionController";
import {selectAppStorage} from "../../runtime/appStorage";
const id = "11111111-1111-4111-8111-111111111111";
const request: AgentWritingPrepareRequest = { conversationId: id, requestId: id, text: "Explain", context: { manuscriptId: id, files: [{relativePath: "main.tex", expectedDiskSha256: "hash"}] } };
const preview: AgentSubmissionPreview = { schemaVersion: 1, conversationId: id, requestId: id, manuscriptId: id, contextSnapshotId: id, turnId: id,
  input: { text: "Explain", model: { providerId: "deepseek", modelId: "deepseek-v4-pro", revision: null }, reasoningEffort: null,
    context: [{sourceId: "file", kind: "manuscript", resourceId: "main.tex", revision: "hash", anchor: JSON.stringify({manuscriptId: id}), text: "original"}] } };
const admission: AgentWritingStartResponse = { schemaVersion: 1, conversationId: id, requestId: id, turnId: id, alreadyStarted: false, outcome: null };
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (reason: Error) => void; const promise = new Promise<T>((a,b) => { resolve = a; reject = b; }); return { resolve, reject, promise }; }
function setup(isAdmissionDisabled?: () => boolean) {
  const values = new Map<string, string>(), storage: Storage = {get length() {return values.size;}, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => {values.set(key, value);}, removeItem: key => {values.delete(key);}, clear: () => values.clear()}; selectAppStorage(storage);
  const gateway = { prepareWriting: vi.fn(async () => preview), prepareWritingStatus: vi.fn(async (intent: AgentWritingPrepareRequest) => ({schemaVersion: 1 as const, intent, status: {type: "notFound" as const}})), restoreWriting: vi.fn(async () => preview), startWriting: vi.fn(async () => admission),
    startStatus: vi.fn(async (intent: AgentWritingStartRequest) => ({schemaVersion: 1 as const, intent, status: {type: "prepared" as const, preview}})) };
  const turns = { interrupt: vi.fn(async () => ({ schemaVersion: 1 as const, conversationId: id, turnId: id, cancellationRequested: true })) };
  return { gateway, turns, storage, controller: new WritingSubmissionController(gateway, turns, undefined, {storage}, isAdmissionDisabled) };
}
async function ready(controller: Pick<WritingSubmissionController, "prepare" | "observeWorkspacePreparation">) {await controller.prepare(request); await controller.observeWorkspacePreparation(id, true);}
it("returns a definitive start rejection to editing without retrying automatically", async () => {
  const {gateway,controller}=setup();
  await ready(controller);
  gateway.startWriting.mockRejectedValueOnce("modelUnavailable");
  await controller.confirm("deepSeekNonThinking");
  expect(controller.getSnapshot()).toMatchObject({phase:"error",error:"startRejected"});
  expect(gateway.startWriting).toHaveBeenCalledOnce();
  expect(await controller.abandonWritingPreparation()).toBe(request.text);
  expect(controller.getSnapshot().phase).toBe("idle");
});
it("keeps the original identity after an uncertain start even when its retry is rejected",async()=>{
  const {gateway,controller}=setup();
  await ready(controller);
  gateway.startWriting.mockRejectedValueOnce(new Error("lost reply")).mockRejectedValueOnce("invalidInput");
  await controller.confirm("deepSeekNonThinking");
  await controller.confirm("deepSeekNonThinking");
  expect(controller.getSnapshot().error).toBe("start");
  expect(controller.dismiss()).toBeNull();
  expect(gateway.startWriting.mock.calls[0]).toEqual(gateway.startWriting.mock.calls[1]);
  expect(gateway.prepareWriting).toHaveBeenCalledOnce();
});
it("keeps an actual ordinary start Unknown through workspace rechecks and forbids returning it to editing", async () => {
  const {gateway, controller} = setup(); await ready(controller);
  gateway.startWriting.mockRejectedValueOnce(new Error("lost start ACK")); await controller.confirm("deepSeekNonThinking");
  await controller.observeWorkspacePreparation(id, false, "conflict");
  expect(controller.getSnapshot()).toMatchObject({phase: "error", error: "start", workspaceFailure: "conflict"});
  expect(await controller.abandonWritingPreparation()).toBeNull(); await controller.retireWritingPreparation();
  await controller.checkWritingPreparation();
  expect(controller.getSnapshot()).toMatchObject({phase: "error", error: "start"}); expect(controller.dismiss()).toBeNull();
  await controller.observeWorkspacePreparation(id, true); await controller.retryWritingPreparation();
  expect(gateway.startWriting.mock.calls[0]).toEqual(gateway.startWriting.mock.calls[1]);
  expect(gateway.prepareWriting).toHaveBeenCalledOnce(); expect(gateway.prepareWritingStatus).not.toHaveBeenCalled();
  expect(await controller.abandonWritingPreparation()).toBeNull(); expect(controller.getSnapshot().phase).toBe("started");
});
it("hands confirmed admission to the workspace after the conversation view detaches", async () => {
  const { gateway, turns } = setup();
  const key = `delivery:${crypto.randomUUID()}`;
  const controller = acquireWritingSubmissionController(key, gateway, turns);
  const admitted = vi.fn(), pending = deferred<AgentWritingStartResponse>();
  await ready(controller);
  gateway.startWriting.mockReturnValue(pending.promise);
  const work = controller.confirm("deepSeekNonThinking", admitted);
  releaseWritingSubmissionController(key, controller);
  pending.resolve(admission);
  await work;
  expect(admitted).toHaveBeenCalledWith({ scope: { manuscriptId: id, conversationId: id, requestId: id, contextSnapshotId: id }, turnId: id });
  expect(turns.interrupt).not.toHaveBeenCalled();
});
it("retains a proven start if the context host detaches before the start response", async () => {
  const {gateway,controller,storage}=setup(), first=vi.fn(async()=>{}), admitted=vi.fn();
  const release=controller.configureContextAcceptance(first), pending=deferred<AgentWritingStartResponse>();
  await ready(controller); gateway.startWriting.mockReturnValue(pending.promise);
  const work=controller.confirm("deepSeekNonThinking",admitted);
  await vi.waitFor(()=>expect(gateway.startWriting).toHaveBeenCalledOnce());
  release(); pending.resolve(admission); await work;
  expect(first).not.toHaveBeenCalled(); expect(admitted).not.toHaveBeenCalled();
  expect(controller.getSnapshot().writingPreparation).toMatchObject({phase:"localError",start:{result:admission}});
  expect(storage.getItem(`research-assistant:writing-prepare:v1:${id}:${id}`)).not.toBeNull();
  const second=vi.fn(async()=>{}); controller.configureContextAcceptance(second);
  await controller.retryWritingPreparation(admitted);
  expect(second).toHaveBeenCalledExactlyOnceWith(id); expect(admitted).toHaveBeenCalledOnce();
  expect(gateway.startWriting).toHaveBeenCalledOnce();
});
it("keeps a later context host registered when an earlier host detaches", async () => {
  const {controller}=setup(), first=vi.fn(async()=>{}), second=vi.fn(async()=>{});
  const releaseFirst=controller.configureContextAcceptance(first);
  controller.configureContextAcceptance(second); releaseFirst();
  await ready(controller); await controller.confirm("deepSeekNonThinking");
  expect(first).not.toHaveBeenCalled(); expect(second).toHaveBeenCalledExactlyOnceWith(id);
});
it("retains editable-copy recovery and its frozen request when the conversation view reopens", async () => {
  const { gateway, turns } = setup();
  const key = `workspace-recovery:${crypto.randomUUID()}`;
  const controller = acquireWritingSubmissionController(key, gateway, turns);
  await controller.prepare(request);
  await controller.observeWorkspacePreparation(id, false);
  releaseWritingSubmissionController(key, controller);
  const reopened = acquireWritingSubmissionController(key, gateway, turns);
  expect(reopened.getSnapshot()).toMatchObject({ phase: "error", error: "workspace", request, preview });
  await reopened.confirm("deepSeekNonThinking");
  expect(gateway.startWriting).not.toHaveBeenCalled();
  await reopened.observeWorkspacePreparation("other", true);
  expect(reopened.getSnapshot().error).toBe("workspace");
  await reopened.observeWorkspacePreparation(id, true);
  await reopened.confirm("deepSeekNonThinking");
  expect(gateway.prepareWriting).toHaveBeenCalledOnce();
  expect(gateway.startWriting).toHaveBeenCalledWith(expect.objectContaining({ requestId: id }));
  forgetWritingSubmissionController(key, reopened);
});

it("does not hand an unconfirmed or mismatched admission to file delivery", async () => {
  const { gateway, controller } = setup(), admitted = vi.fn();
  await ready(controller);
  gateway.startWriting.mockRejectedValueOnce(new Error("lost response"));
  await controller.confirm("deepSeekNonThinking", admitted);
  expect(admitted).not.toHaveBeenCalled();
  gateway.startWriting.mockResolvedValueOnce({ ...admission, requestId: "other" });
  await controller.confirm("deepSeekNonThinking", admitted);
  expect(admitted).not.toHaveBeenCalled();
  expect(controller.getSnapshot()).toMatchObject({ phase: "error", error: "start" });
});
it("freezes the input, deduplicates prepare and ignores a disposed late preview without discarding Unknown", async () => {
  const { gateway, controller } = setup();
  const pending = deferred<AgentSubmissionPreview>(); gateway.prepareWriting.mockReturnValue(pending.promise);
  const input = structuredClone(request); const work = controller.prepare(input);
  (input as { text: string }).text = "new text";
  await controller.prepare(input);
  await vi.waitFor(() => expect(gateway.prepareWriting).toHaveBeenCalledTimes(1));
  expect(controller.dismiss()).toBeNull(); controller.dispose();
  pending.resolve(preview); await work;
  expect(controller.getSnapshot().phase).toBe("preparing");
  expect(gateway.startWriting).not.toHaveBeenCalled();
});
it("discards only an unstarted failed preparation and allows a fresh user submission", async () => {
  const { gateway, controller } = setup(); gateway.prepareWriting.mockRejectedValueOnce("modelUnavailable");
  await controller.prepare(request); expect(controller.getSnapshot().request?.text).toBe("Explain");
  await controller.checkWritingPreparation();
  await controller.retireWritingPreparation();
  expect(gateway.prepareWriting).toHaveBeenCalledOnce();
  expect(gateway.restoreWriting).not.toHaveBeenCalled();
  expect(controller.getSnapshot().writingPreparation?.phase).toBe("retired");
  expect(controller.getSnapshot().phase).toBe("idle");
  await controller.prepare({...request, requestId: "22222222-2222-4222-8222-222222222222", text: "Revised"});
  expect(gateway.prepareWriting).toHaveBeenCalledTimes(2);
  expect(gateway.prepareWriting).toHaveBeenLastCalledWith(expect.objectContaining({text: "Revised", requestId: "22222222-2222-4222-8222-222222222222"}));
  expect(gateway.startWriting).not.toHaveBeenCalled();
});
it("retries an ambiguous start with its original identity and waits for durable terminal history", async () => {
  const { gateway, turns, controller } = setup();
  await ready(controller); gateway.startWriting.mockRejectedValueOnce(new Error("lost"));
  await controller.confirm("deepSeekNonThinking"); expect(controller.dismiss()).toBeNull();
  await controller.confirm("deepSeekNonThinking");
  expect(gateway.startWriting.mock.calls[0]).toEqual(gateway.startWriting.mock.calls[1]);
  await controller.stop(); await controller.stop(); expect(turns.interrupt).toHaveBeenCalledTimes(1);
  expect(controller.getSnapshot().outcome).toBeNull(); expect(controller.dismiss()).toBeNull();
  controller.observeOutcome(id, "canceled"); expect(controller.getSnapshot().outcome).toBe("canceled");
  expect(controller.dismiss()).toBe("Explain");
});
it("retains a terminal event arriving before admission and never cancels on disposal", async () => {
  const { gateway, turns, controller } = setup(); await ready(controller);
  const pending = deferred<AgentWritingStartResponse>(); gateway.startWriting.mockReturnValue(pending.promise);
  const work = controller.confirm("deepSeekNonThinking"); controller.observeOutcome(id, "completed");
  pending.resolve(admission); await work; expect(controller.getSnapshot().outcome).toBe("completed");
  expect(controller.dismiss()).toBeNull(); expect(controller.getSnapshot().phase).toBe("idle");
  controller.dispose(); expect(turns.interrupt).not.toHaveBeenCalled();
});

it("keeps an in-flight prepare alive when the conversation view switches", async () => {
  const { gateway, controller } = setup();
  const pending = deferred<AgentSubmissionPreview>();
  gateway.prepareWriting.mockReturnValue(pending.promise);
  const work = controller.prepare(request);

  // The keyed writing panel is unmounted when the user switches conversations.
  // Detaching its view must not fence the owner that is still preparing.
  controller.detach();
  pending.resolve(preview);
  await work;

  expect(controller.getSnapshot()).toMatchObject({ phase: "preview", preview });
});

it("reattaches a switched conversation to its existing submission owner", async () => {
  const { gateway, turns } = setup();
  const key = `test:${crypto.randomUUID()}`;
  const first = acquireWritingSubmissionController(key, gateway, turns);
  const pending = deferred<AgentSubmissionPreview>();
  gateway.prepareWriting.mockReturnValue(pending.promise);
  const work = first.prepare(request);
  releaseWritingSubmissionController(key, first);

  const second = acquireWritingSubmissionController(key, gateway, turns);
  expect(second).toBe(first);
  pending.resolve(preview);
  await work;
  expect(second.getSnapshot()).toMatchObject({ phase: "preview", preview });
  forgetWritingSubmissionController(key, second);
});

it("queries pending pooled submissions through detach without preparing, replaying or releasing them", async () => {
  resetWritingSubmissionControllersForTests();
  const {gateway, turns} = setup();
  const key = "opaque pending owner key";
  const controller = acquireWritingSubmissionController(key, gateway, turns);
  try {
    expect(hasPendingWritingSubmissions()).toBe(false);
    const pending = deferred<AgentSubmissionPreview>();
    gateway.prepareWriting.mockReturnValueOnce(pending.promise);
    const work = controller.prepare(request);
    await vi.waitFor(() => expect(gateway.prepareWriting).toHaveBeenCalledOnce());
    releaseWritingSubmissionController(key, controller);
    const snapshot = controller.getSnapshot();
    expect(hasPendingWritingSubmissions()).toBe(true);
    expect(hasPendingWritingSubmissions()).toBe(true);
    expect(controller.getSnapshot()).toBe(snapshot);
    expect(gateway.prepareWriting).toHaveBeenCalledOnce();
    expect(gateway.prepareWritingStatus).not.toHaveBeenCalled();
    expect(gateway.startWriting).not.toHaveBeenCalled();
    expect(turns.interrupt).not.toHaveBeenCalled();
    const reattached = acquireWritingSubmissionController(key, gateway, turns);
    expect(reattached).toBe(controller);
    pending.resolve(preview); await work;
    expect(hasPendingWritingSubmissions()).toBe(true);
    await controller.observeWorkspacePreparation(id, true);
    await controller.confirm("deepSeekNonThinking");
    expect(controller.getSnapshot().phase).toBe("started");
    expect(hasPendingWritingSubmissions()).toBe(false);
    releaseWritingSubmissionController(key, reattached);
    expect(hasPendingWritingSubmissions()).toBe(false);
  } finally { resetWritingSubmissionControllersForTests(); }
});

it("retains the original stop and storage-recovery qualifications in pending queries", async () => {
  resetWritingSubmissionControllersForTests();
  const {gateway, turns, storage} = setup();
  const key = "stop owner";
  const controller = acquireWritingSubmissionController(key, gateway, turns);
  try {
    await ready(controller); await controller.confirm("deepSeekNonThinking");
    expect(hasPendingWritingSubmissions()).toBe(false);
    const response = deferred<Awaited<ReturnType<typeof turns.interrupt>>>();
    turns.interrupt.mockReturnValueOnce(response.promise);
    const stopping = controller.stop();
    const snapshot = controller.getSnapshot();
    expect(hasPendingWritingSubmissions()).toBe(true);
    expect(controller.getSnapshot()).toBe(snapshot);
    expect(turns.interrupt).toHaveBeenCalledOnce();
    response.resolve({schemaVersion: 1, conversationId: id, turnId: id, cancellationRequested: true});
    await stopping;
    expect(hasPendingWritingSubmissions()).toBe(false);
    releaseWritingSubmissionController(key, controller);

    const getItem = vi.spyOn(storage, "getItem").mockImplementation(key => {
      if (key.startsWith("research-assistant:writing-queue-stop:") || key.startsWith("research-assistant:writing-send-stop:")) throw new Error("storage unavailable");
      return null;
    });
    const recovered = acquireWritingSubmissionController("recovery owner", gateway, turns, undefined,
      {scope: {manuscriptId: id, conversationId: id}, storage});
    expect(recovered.getSnapshot()).toMatchObject({phase: "idle", queueStopHoldUnavailable: true, sendStopIntentUnavailable: true});
    const readCount = getItem.mock.calls.length;
    expect(hasPendingWritingSubmissions()).toBe(true);
    expect(getItem).toHaveBeenCalledTimes(readCount);
    releaseWritingSubmissionController("recovery owner", recovered);
    expect(hasPendingWritingSubmissions()).toBe(true);
  } finally { resetWritingSubmissionControllersForTests(); }
});

it("keeps an idle controller pending only while its original queue hold is unresolved", async () => {
  resetWritingSubmissionControllersForTests();
  const {gateway, turns, storage} = setup();
  const controller = acquireWritingSubmissionController("queue hold", gateway, turns, undefined, {storage});
  const scope = {manuscriptId: id, conversationId: id};
  try {
    expect(hasPendingWritingSubmissions()).toBe(false);
    await controller.holdQueueInput(scope, id);
    expect(controller.getSnapshot()).toMatchObject({phase: "idle", heldQueueInputId: id});
    const snapshot = controller.getSnapshot(), storageLength = storage.length;
    const storageKey = `research-assistant:writing-queue-stop:v1:${id}:${id}`;
    const stored = storage.getItem(storageKey);
    expect(hasPendingWritingSubmissions()).toBe(true);
    expect(controller.getSnapshot()).toBe(snapshot);
    expect(storage.length).toBe(storageLength);
    expect(storage.getItem(storageKey)).toBe(stored);
    expect(await controller.clearQueueInputHold(scope, id)).toBe(true);
    expect(hasPendingWritingSubmissions()).toBe(false);
    releaseWritingSubmissionController("queue hold", controller);
  } finally { resetWritingSubmissionControllersForTests(); }
});

it("keeps the first owner's captured admission getter when a later view supplies another one", async () => {
  resetWritingSubmissionControllersForTests();
  const {gateway, turns, storage} = setup();
  const first = acquireWritingSubmissionController("captured admission", gateway, turns, undefined, {storage}, () => true);
  const second = acquireWritingSubmissionController("captured admission", gateway, turns, undefined, {storage}, () => false);
  expect(second).toBe(first); await second.prepare(request);
  expect(gateway.prepareWriting).not.toHaveBeenCalled();
  expect(second.getSnapshot().writingPreparation?.cause).toMatchObject({code: "WRITING_DISPATCH_ADMISSION_CLOSED"});
  releaseWritingSubmissionController("captured admission", second); releaseWritingSubmissionController("captured admission", first);
  resetWritingSubmissionControllersForTests();
});

it("retires a warm unissued start after an earlier Unknown preparation became committed", async () => {
  let disabled = false; const f = setup(() => disabled);
  f.gateway.prepareWriting.mockRejectedValueOnce(new Error("prepare ACK lost"));
  await f.controller.prepare(request); await f.controller.retryWritingPreparation();
  expect(f.controller.getSnapshot().writingPreparation).toMatchObject({priorUnknown: true, knownCommitted: true});
  await f.controller.observeWorkspacePreparation(id, true);
  const {registerStoragePersistence} = await import("../../runtime/storagePersistence");
  let release!: () => void; const gate = new Promise<void>(resolve => {release = resolve;});
  registerStoragePersistence(f.storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush: () => gate, retry: async () => {}, reset: async () => {}, subscribe: () => () => {}, close: () => {}});
  const work = f.controller.confirm("deepSeekNonThinking");
  await vi.waitFor(() => expect(f.controller.getSnapshot().writingPreparation?.start).not.toBeNull());
  disabled = true; release(); await work;
  expect(f.gateway.startWriting).not.toHaveBeenCalled();
  expect(await f.controller.cancelPendingSend(id)).toBe(true);
  expect(f.controller.getSnapshot().writingPreparation?.phase).toBe("retired");
  expect(f.controller.isCancelPending(id)).toBe(false);
});
