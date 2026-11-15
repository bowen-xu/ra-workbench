import {expect, it, vi} from "vitest";
import {decodeAgentWritingQueueCancelStatusResponse, type AgentSubmissionPreview, type AgentWritingPrepareRequest} from "@research-assistant/core/agent";
import {WritingDiffDemoQueue} from "./WritingDiffDemoQueue";
import type {AgentSubmissionGateway} from "@research-assistant/data/agent";

async function fixture() {
  const scope = {conversationId: crypto.randomUUID(), manuscriptId: crypto.randomUUID()}, inputId = crypto.randomUUID();
  const submission = {prepareWriting: vi.fn(async (request: AgentWritingPrepareRequest): Promise<AgentSubmissionPreview> => ({
    schemaVersion: 1, ...scope, requestId: request.requestId, turnId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID(),
    input: {text: request.text, reasoningEffort: null, model: {providerId: "fixture", modelId: "model", revision: null}, context: []},
  })), prepareWritingStatus: vi.fn(), startStatus: vi.fn<AgentSubmissionGateway["startStatus"]>(async (): Promise<never> => {throw new Error("queue cancellation does not inspect ordinary starts");}), restoreWriting: vi.fn(), startWriting: vi.fn()};
  const queue = new WritingDiffDemoQueue(submission), addOp = crypto.randomUUID();
  const added = await queue.gateway.edit({...scope, requestId: addOp, expectedRevision: 0, edit: {type: "add", inputId, text: "retained"}});
  const prepared = await queue.preview.prepare({inputId, expectedRevision: added.revision,
    submission: {conversationId: scope.conversationId, requestId: crypto.randomUUID(), text: "retained", context: {manuscriptId: scope.manuscriptId, files: []}}});
  const intent = {...scope, requestId: crypto.randomUUID(), inputId, expectedRevision: prepared.queue.revision, previewRequestId: prepared.preview.requestId};
  return {scope, queue, submission, intent, addOp};
}

it("keeps the committed cancel proof across snapshot restore while later queue data remains current", async () => {
  const f = await fixture(); f.queue.failNextCancel("afterCommit");
  await expect(f.queue.preview.cancel(f.intent)).rejects.toThrow("lost cancellation");
  const status = decodeAgentWritingQueueCancelStatusResponse(await f.queue.preview.cancelStatus(f.intent));
  expect(status.status).toMatchObject({type: "committed", proof: {family: "agent.queue.preview.cancel", operationId: f.intent.requestId},
    queue: {paused: true, preparingInputId: null}});
  const current = await f.queue.gateway.read(f.scope);
  const newer = await f.queue.gateway.edit({...f.scope, requestId: crypto.randomUUID(), expectedRevision: current.revision,
    edit: {type: "add", inputId: crypto.randomUUID(), text: "new tail"}});
  const reopened = new WritingDiffDemoQueue(f.submission); reopened.restore(f.queue.snapshot());
  expect(await reopened.preview.cancelStatus(f.intent)).toEqual(status);
  expect(await reopened.preview.cancel(f.intent)).toEqual(status.status.type === "committed" ? status.status.queue : null);
  expect(await reopened.gateway.read(f.scope)).toEqual(newer); expect(f.submission.startWriting).not.toHaveBeenCalled();
});

it("checks the entire original cancel intent and rejects cross-family operations without deleting the preview", async () => {
  const f = await fixture();
  await expect(f.queue.preview.cancel({...f.intent, requestId: f.addOp})).rejects.toMatchObject({code: "storage", reason: {code: "DATA_SQL_DOMAIN", reason: "conversation changed since it was read"}});
  await f.queue.preview.cancel(f.intent);
  for (const patch of [{expectedRevision: f.intent.expectedRevision + 1}, {inputId: crypto.randomUUID()}, {previewRequestId: crypto.randomUUID()}])
    await expect(f.queue.preview.cancelStatus({...f.intent, ...patch})).rejects.toMatchObject({code: "storage", reason: {code: "DATA_SQL_DOMAIN"}});
});

it("status is read-only during an unavailable owner and notFound does not mutate a prepared queue", async () => {
  const f = await fixture(), before = await f.queue.gateway.read(f.scope);
  expect((await f.queue.preview.cancelStatus(f.intent)).status.type).toBe("notFound");
  f.queue.failNextCancel("unavailable");
  expect(decodeAgentWritingQueueCancelStatusResponse(await f.queue.preview.cancelStatus(f.intent)).status.type).toBe("unconfirmed");
  expect(await f.queue.gateway.read(f.scope)).toEqual(before); expect(f.submission.startWriting).not.toHaveBeenCalled();
});

it("removes cancellation receipts and their historical queue payload when the conversation is deleted", async () => {
  const f = await fixture(); await f.queue.preview.cancel(f.intent);
  expect(f.queue.snapshot().cancellations.size).toBe(1); f.queue.deleteConversation(f.scope.conversationId);
  expect(f.queue.snapshot().cancellations.size).toBe(0);
});
