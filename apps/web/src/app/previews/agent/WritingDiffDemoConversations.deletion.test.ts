import { afterEach, expect, it } from "vitest";
import { createWritingDiffDemoRuntime } from "./writingDiffDemoRuntime";
import { WritingDiffDemoStorage } from "./WritingDiffDemoStorage";
import { WritingDiffDemoAgentStorage } from "./WritingDiffDemoAgentStorage";
import { initialChanges } from "../../../core/features/agent/WritingDiffFixtures";
import type { AgentDeletionRequest } from "@research-assistant/core/agent";

const live: ReturnType<typeof createWritingDiffDemoRuntime>[] = [];
afterEach(() => { for (const runtime of live.splice(0)) runtime.dispose(); });
async function fixture() {
  const manuscriptId = crypto.randomUUID(), conversationId = crypto.randomUUID();
  const admission = { scope: { manuscriptId, conversationId, requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
  const storage = new WritingDiffDemoStorage(manuscriptId, initialChanges);
  const agent = new WritingDiffDemoAgentStorage(storage, { admission, changes: initialChanges });
  const runtime = createWritingDiffDemoRuntime(storage, agent, manuscriptId, conversationId);
  live.push(runtime); await agent.initialized;
  const binding = { kind: "manuscript" as const, resourceId: manuscriptId };
  await runtime.conversations.archives.setArchived({operationId: crypto.randomUUID(), id: conversationId, archived: true, expectedWatermark: (await runtime.conversations.metadata.read({id: conversationId})).durableCount});
  const current = await runtime.conversations.metadata.read({ id: conversationId, binding });
  const request: AgentDeletionRequest & {binding: typeof binding} = { schemaVersion: 1, requestId: crypto.randomUUID(), conversationId, binding,
    expectedTitle: current.title, expectedDurableCount: current.durableCount, force: true };
  return { storage, agent, runtime, request, admission };
}
it("uses durable deletion receipts, clears chat history and fences stale creators", async () => {
  const f = await fixture();
  const before = await f.storage.files.readFile(f.request.binding.resourceId, "main.tex");
  await expect(f.runtime.listGateways.deletions.delete({ ...f.request, force: false })).rejects.toBe("executionWorkspace");
  const receipt = await f.runtime.listGateways.deletions.delete(f.request);
  expect(receipt).toEqual({ schemaVersion: 1, requestId: f.request.requestId, conversationId: f.request.conversationId });
  expect(await f.runtime.listGateways.deletions.delete(f.request)).toEqual(receipt);
  await expect(f.runtime.listGateways.deletions.delete({ ...f.request, expectedTitle: "different" }))
    .rejects.toMatchObject({code: "storage", reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
  await expect(f.runtime.conversations.metadata.read({ id: f.request.conversationId, binding: f.request.binding })).rejects.toBe("notFound");
  await expect(f.agent.gateways.delivery.sessions.open(f.request.conversationId)).rejects.toBe("notFound");
  await expect(f.runtime.conversations.creation.createBound({ operationId: crypto.randomUUID(), id: f.request.conversationId, binding: f.request.binding })).rejects.toBe("notFound");
  expect(await f.storage.files.readFile(f.request.binding.resourceId, "main.tex")).toEqual(before);
  expect((await f.agent.gateways.delivery.deliveries.list({ manuscriptId: f.request.binding.resourceId, after: null, limit: 64 })).items).toHaveLength(0);
});
it("batches accepted deletions and retains rejected conversations in the isolated preview", async () => {
  const f = await fixture(), otherId = crypto.randomUUID();
  await f.runtime.conversations.creation.createBound({operationId: crypto.randomUUID(), id: otherId, binding: f.request.binding});
  const other = await f.runtime.conversations.metadata.read({id: otherId, binding: f.request.binding});
  const response = await f.runtime.listGateways.batchDeletions.deleteBatch([f.request, {...f.request, requestId: crypto.randomUUID(),
    conversationId: otherId, expectedTitle: other.title, expectedDurableCount: other.durableCount}]);
  expect(response.results.map(result => result.status)).toEqual(["deleted", "failed"]);
  expect(response.results[1]).toMatchObject({error: "notArchived"});
  await expect(f.runtime.conversations.metadata.read({id: f.request.conversationId})).rejects.toBe("notFound");
  expect((await f.runtime.conversations.metadata.read({id: otherId})).id).toBe(otherId);
});
it("checks archive, binding, title and watermark before deletion without clearing history", async () => {
  const f = await fixture();
  for (const [patch, error] of [
    [{ expectedTitle: "stale" }, "conflict"], [{ expectedDurableCount: 0 }, "conflict"],
    [{ binding: { kind: "manuscript", resourceId: crypto.randomUUID() } }, "notFound"],
  ] as const) await expect(f.runtime.listGateways.deletions.delete({ ...f.request, ...patch })).rejects.toBe(error);
  await f.runtime.conversations.archives.setArchived({operationId: crypto.randomUUID(), id: f.request.conversationId, archived: false, expectedWatermark: null});
  await expect(f.runtime.listGateways.deletions.delete(f.request)).rejects.toBe("notArchived");
  expect((await f.agent.gateways.delivery.sessions.open(f.request.conversationId)).watermark).toBe(f.request.expectedDurableCount);
});

it("replays concurrent identical delete requests and prevents stale queued work from recreating history", async () => {
  const f = await fixture();
  const [first, second] = await Promise.all([f.runtime.listGateways.deletions.delete(f.request), f.runtime.listGateways.deletions.delete(f.request)]);
  expect(first).toEqual(second);
  await expect(f.agent.complete(f.admission, initialChanges)).rejects.toBe("notFound");
  await expect(f.runtime.gateways.queue.gateway.read({ manuscriptId: f.request.binding.resourceId, conversationId: f.request.conversationId })).rejects.toBe("notFound");
  await expect(f.runtime.conversations.archives.setArchived({operationId: crypto.randomUUID(), id: f.request.conversationId, archived: false, expectedWatermark: null})).rejects.toBe("notFound");
});
it("refuses archive and forced deletion of a running turn without losing history", async () => {
  const f = await fixture();
  const running = { scope: { ...f.admission.scope, requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
  f.agent.history.start(running, "still running");
  const current = await f.agent.gateways.delivery.sessions.open(f.request.conversationId);
  await expect(f.runtime.conversations.archives.setArchived({operationId: crypto.randomUUID(), id: f.request.conversationId, archived: true, expectedWatermark: (await f.runtime.conversations.metadata.read({id: f.request.conversationId})).durableCount})).rejects.toBe("busy");
  await expect(f.runtime.listGateways.deletions.delete({ ...f.request, expectedDurableCount: current.watermark })).rejects.toBe("busy");
  expect(await f.agent.gateways.delivery.sessions.open(f.request.conversationId)).toEqual(current);
});

it("keeps delivery pagination monotonic after deleting an unadmitted proposal", async () => {
  const f = await fixture(), manuscriptId = f.request.binding.resourceId;
  const first = await f.agent.gateways.delivery.deliveries.list({ manuscriptId, after: null, limit: 64 });
  await f.runtime.listGateways.deletions.delete(f.request);
  const other = { scope: { manuscriptId, conversationId: crypto.randomUUID(), requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
  await f.agent.complete(other, initialChanges);
  const next = await f.agent.gateways.delivery.deliveries.list({ manuscriptId, after: first.items[0]!.id, limit: 64 });
  expect(next.items).toHaveLength(1); expect(next.items[0]!.scope).toEqual(other.scope);
  expect(next.items[0]!.id).toBeGreaterThan(first.items[0]!.id);
});

it("retains the original epoch across await and refuses a swapped mock owner before history deletion", async () => {
  const f = await fixture(), snapshot = f.runtime.conversations.snapshot(), oldEpoch = snapshot.renames.epoch;
  const newEpoch = crypto.randomUUID(), options = {expectedEpoch: oldEpoch}, before = f.agent.history.snapshot();
  const pending = f.runtime.listGateways.deletions.delete(f.request, options);
  const rejected = expect(pending).rejects.toMatchObject({reason: {reason: {phase: "delete-epoch", safeCause: "data_epoch_changed"}}});
  options.expectedEpoch = newEpoch;
  // The replacement data set has identical rows, but independent operation evidence.
  const empty = {epoch: newEpoch, entries: new Map()};
  f.runtime.conversations.restore({...snapshot, renames: empty, creations: {entries: new Map()}, archiveReceipts: empty,
    forkReceipts: empty, deletionReceipts: empty});
  await rejected;
  expect(f.agent.history.snapshot()).toEqual(before);
  expect(f.runtime.conversations.snapshot().records.has(f.request.conversationId)).toBe(true);
  expect((await f.runtime.listGateways.deletions.status(f.request)).status).toMatchObject({type: "noSqlReceipt", epoch: newEpoch});
  await f.runtime.listGateways.deletions.delete(f.request, {expectedEpoch: newEpoch});
  expect((await f.runtime.listGateways.deletions.status(f.request)).status.type).toBe("committed");
});
