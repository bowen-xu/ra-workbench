import { expect, it, vi } from "vitest";
import type { AgentSubmissionPreview, AgentWritingPrepareRequest, AgentWritingQueueResponse, AgentWritingStartRequest } from "@research-assistant/core/agent";
import { WritingSubmissionController } from "./writingSubmissionController";
const id = "11111111-1111-4111-8111-111111111111";
const other = "22222222-2222-4222-8222-222222222222";
const request: AgentWritingPrepareRequest = { conversationId: id, requestId: id, text: "Queued", context: { manuscriptId: id, files: [] } };
const preview: AgentSubmissionPreview = { schemaVersion: 1, conversationId: id, requestId: id, manuscriptId: id, contextSnapshotId: id, turnId: id,
  input: { text: "Queued", model: { providerId: "deepseek", modelId: "deepseek-v4-pro", revision: null }, reasoningEffort: null,
    context: [{sourceId: "file", kind: "manuscript", resourceId: "main.tex", revision: "hash", anchor: JSON.stringify({manuscriptId: id}), text: "original"}] } };
const queue: AgentWritingQueueResponse = { schemaVersion: 1, conversationId: id, manuscriptId: id, revision: 2,
  paused: false, preparingInputId: other, inputs: [{ inputId: other, text: "Queued" }] };
function setup() {
  const admission = { schemaVersion: 1 as const, conversationId: id, requestId: id, turnId: id, alreadyStarted: false, outcome: null };
  const ordinary = { prepareWriting: vi.fn(async () => preview), prepareWritingStatus: vi.fn(), restoreWriting: vi.fn(async () => preview), startWriting: vi.fn(async () => admission),
    startStatus: vi.fn(async (intent: AgentWritingStartRequest) => ({schemaVersion: 1 as const, intent, status: {type: "prepared" as const, preview}})) };
  const queued = { prepareStatus: vi.fn(async (intent: import("@research-assistant/core/agent").AgentWritingQueuePreviewRequest) => ({schemaVersion: 1 as const, intent, status: {type: "committed" as const, result: {schemaVersion: 1 as const, inputId: other, preview, queue}, proof: {schemaVersion: 1 as const, operationId: intent.submission.requestId, conversationId: intent.submission.conversationId, family: "agent.queue.preview.prepare" as const, epoch: id, sqlFence: "2", replayed: true, result: null}}})), recover: vi.fn(async () => ({ schemaVersion: 1 as const, inputId: other, preview, queue })), prepare: vi.fn(async () => ({ schemaVersion: 1 as const, inputId: other, preview, queue })),
    cancel: vi.fn(async () => ({ ...queue, revision: 3, paused: true, preparingInputId: null })),
    cancelStatus: vi.fn(async (intent: import("@research-assistant/core/agent").AgentWritingQueueCancelRequest) => ({schemaVersion: 1 as const, intent, status: {type: "notFound" as const}})),
    startStatus: vi.fn(async (intent: import("@research-assistant/core/agent").AgentWritingQueuedStartRequest) => ({schemaVersion: 1 as const, intent, status: {type: "prepared" as const, preview}})), start: vi.fn(async () => admission) };
  const turns = { interrupt: vi.fn(async () => ({ schemaVersion: 1 as const, conversationId: id, turnId: id, cancellationRequested: true })) };
  const values = new Map<string, string>(), storage: Storage = {get length() {return values.size;}, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => {values.set(key, value);}, removeItem: key => {values.delete(key);}, clear: () => values.clear()};
  return { ordinary, queued, controller: new WritingSubmissionController(ordinary, turns, queued, {storage}) };
}
it("prepares and confirms a queued input only through the queue gateways", async () => {
  const { ordinary, queued, controller } = setup();
  await controller.prepareQueued(request, other, 1);
  await controller.observeWorkspacePreparation(id, true);
  expect(controller.getSnapshot().queue).toEqual(queue);
  expect(controller.dismiss()).toBeNull();
  queued.start.mockRejectedValueOnce(new Error("lost"));
  await controller.confirm("deepSeekNonThinking");
  expect(controller.dismiss()).toBeNull();
  await controller.cancelQueued(id);
  expect(queued.cancel).not.toHaveBeenCalled();
  await controller.confirm("deepSeekNonThinking");
  expect(queued.start.mock.calls[0]).toEqual(queued.start.mock.calls[1]);
  expect(queued.start).toHaveBeenCalledWith({ inputId: other, expectedRevision: 2, start: {
    conversationId: id, requestId: id, manuscriptId: id, contextSnapshotId: id, model: preview.input.model, mode: "deepSeekNonThinking" } });
  expect(ordinary.prepareWriting).not.toHaveBeenCalled();
  expect(ordinary.startWriting).not.toHaveBeenCalled();
  controller.observeOutcome(id, "completed"); controller.dismiss();
  await controller.prepare(request); expect(ordinary.prepareWriting).toHaveBeenCalledOnce();
});
it("retains the queue identity after uncertain preparation and reconciles the original request", async () => {
  const { ordinary, queued, controller } = setup();
  queued.prepare.mockRejectedValueOnce(new Error("lost"));
  const input = structuredClone(request);
  await controller.prepareQueued(input, other, 1);
  (input as { text: string }).text = "Later draft";
  expect(controller.dismiss()).toBeNull();
  await controller.restoreQueuedPreparation();
  expect(queued.prepare).toHaveBeenCalledOnce();
  expect(queued.recover).toHaveBeenCalledWith({ conversationId: id, manuscriptId: id });
  expect(ordinary.restoreWriting).not.toHaveBeenCalled();
  expect(controller.getSnapshot().preview?.input.text).toBe("Queued");
});
it("retries cancellation with the same receipt and returns the paused retained head", async () => {
  const { queued, controller } = setup();
  await controller.prepareQueued(request, other, 1);
  queued.cancel.mockRejectedValueOnce(new Error("lost"));
  await controller.cancelQueued(id);
  expect(controller.getSnapshot().error).toBe("cancel");
  expect(controller.dismiss()).toBeNull();
  await controller.confirm("deepSeekNonThinking"); expect(queued.start).not.toHaveBeenCalled();
  await controller.cancelQueued(other);
  expect(queued.cancel.mock.calls[0]).toEqual(queued.cancel.mock.calls[1]);
  expect(controller.getSnapshot()).toMatchObject({ phase: "idle", queue: null });
});
it("ignores cancellation completion after disposal without canceling or starting a run", async () => {
  const { queued, controller } = setup();
  await controller.prepareQueued(request, other, 1);
  let resolve!: (value: Awaited<ReturnType<typeof queued.cancel>>) => void;
  queued.cancel.mockReturnValue(new Promise((done) => { resolve = done; }));
  const work = controller.cancelQueued(id);
  await controller.cancelQueued(other);
  await vi.waitFor(() => expect(queued.cancel).toHaveBeenCalledOnce());
  controller.dispose(); resolve({ ...queue, revision: 3, paused: true, preparingInputId: null }); await work;
  expect(controller.getSnapshot().phase).toBe("canceling");
  expect(queued.start).not.toHaveBeenCalled();
});
it("recovers frozen preview without preparing files and confirms with the current queue revision", async () => {
  const { ordinary, queued, controller } = setup();
  queued.recover.mockResolvedValue({ schemaVersion: 1, inputId: other, preview, queue: { ...queue, revision: 8 } });
  await controller.recoverQueued({ conversationId: id, manuscriptId: id });
  expect(controller.getSnapshot()).toMatchObject({ phase: "preview", request: null, preview });
  expect(queued.prepare).not.toHaveBeenCalled(); expect(ordinary.prepareWriting).not.toHaveBeenCalled();
  expect(queued.start).not.toHaveBeenCalled(); expect(controller.dismiss()).toBeNull();
  await controller.confirm("deepSeekNonThinking");
  expect(queued.start).toHaveBeenCalledWith(expect.objectContaining({ inputId: other, expectedRevision: 8 }));
});
it("can retry a failed recovery and cancel the recovered claim", async () => {
  const { queued, controller } = setup();
  queued.recover.mockRejectedValueOnce(new Error("offline"));
  await controller.recoverQueued({ conversationId: id, manuscriptId: id });
  expect(controller.getSnapshot().error).toBe("recover");
  await controller.recoverQueued({ conversationId: id, manuscriptId: id });
  await controller.cancelQueued(id);
  expect(queued.cancel).toHaveBeenCalledWith(expect.objectContaining({ inputId: other, previewRequestId: id, expectedRevision: 2 }));
  expect(controller.getSnapshot().phase).toBe("idle");
});
it("does not substitute a different preview when reconciling an uncertain prepare", async () => {
  const { queued, controller } = setup();
  queued.prepare.mockRejectedValueOnce(new Error("lost"));
  await controller.prepareQueued(request, other, 1);
  queued.recover.mockResolvedValue({ schemaVersion: 1, inputId: other, preview: { ...preview, requestId: other }, queue });
  await controller.restoreQueuedPreparation();
  expect(controller.getSnapshot()).toMatchObject({ phase: "idle", error: null, preview: null });
  expect(queued.start).not.toHaveBeenCalled(); expect(controller.dismiss()).toBeNull();
});
it("reviews an admitted record and retries its original identity without requeue or preparation", async () => {
  const { queued, ordinary, controller } = setup();
  const candidate = { inputId: other, expectedRevision: 9, preview: structuredClone(preview) };
  controller.reviewUnstartedAdmission(candidate);
  (candidate.preview.input as { text: string }).text = "Changed caller object";
  expect(controller.getSnapshot().preview?.input.text).toBe("Queued");
  expect(queued.start).not.toHaveBeenCalled();
  controller.dismiss(); expect(controller.getSnapshot().phase).toBe("idle");
  expect(queued.cancel).not.toHaveBeenCalled();
  controller.reviewUnstartedAdmission({ inputId: other, expectedRevision: 9, preview });
  queued.start.mockRejectedValueOnce(new Error("lost"));
  await controller.confirm("deepSeekNonThinking"); expect(controller.dismiss()).toBeNull();
  await controller.confirm("deepSeekNonThinking");
  expect(queued.start.mock.calls[0]).toEqual(queued.start.mock.calls[1]);
  expect(queued.start).toHaveBeenCalledWith(expect.objectContaining({ inputId: other, expectedRevision: 9 }));
  expect(queued.prepare).not.toHaveBeenCalled(); expect(ordinary.startWriting).not.toHaveBeenCalled();
});
