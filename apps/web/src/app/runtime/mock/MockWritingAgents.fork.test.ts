// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { MockWritingAgents } from "./MockWritingAgents";
import { createMockWritingGateway } from "./mockWritingGateway";
import { createBrowserWritingCheckpointGateway } from "../../../../../../plugins/writing_projects/src/features/writing/gateway/browserWritingCheckpointGateway";
import { createPreviewControls } from "../../../core/runtime/previewControls";
import { createTestNamespacedStorage } from "../../../core/runtime/testing/createTestNamespacedStorage";
import { registerWritingRecords } from "../../../core/features/writing-storage/browserWritingRecords";
import { memoryWritingRecords, memoryWritingState } from "../testing/memoryWritingRecords";
const live: MockWritingAgents[] = [];
afterEach(() => { for (const owner of live.splice(0)) owner.dispose(); });
function snapshot(storage: Storage) { return new Map(Array.from({ length: storage.length }, (_, index) => { const key = storage.key(index)!; return [key, storage.getItem(key)]; })); }
it("reads the same fork proof after cold reopen and resource deletion without opening a runtime or rewriting storage", async () => {
  const storage = createTestNamespacedStorage(localStorage, `${crypto.randomUUID()}:`);
  registerWritingRecords(storage, memoryWritingRecords(memoryWritingState()));
  const writing = createMockWritingGateway(storage), manuscript = await writing.createManuscript({ title: "Synthetic" });
  const controls = createPreviewControls(storage, () => undefined); controls.configure({ delayMs: 0, nextFailure: null });
  const open = () => { const value = new MockWritingAgents(storage, writing, createBrowserWritingCheckpointGateway(storage), controls); live.push(value); return value; };
  const owner = open(), ports = owner.gateways(), parentId = crypto.randomUUID(), binding = { kind: "manuscript" as const, resourceId: manuscript.id };
  await ports.agentCreationGateway!.createBound({ operationId: crypto.randomUUID(), id: parentId, binding });
  const backend = owner.backend(manuscript.id), input = { scope: { manuscriptId: manuscript.id, conversationId: parentId, requestId: crypto.randomUUID(), contextSnapshotId: crypto.randomUUID() }, turnId: crypto.randomUUID() };
  backend.agent.history.start(input, "Synthetic fork source"); await backend.agent.complete(input, []);
  const request = { childId: crypto.randomUUID(), parentId, throughTurnId: input.turnId, binding };
  const before = await ports.agentForkGateway!.status(request);
  const child = await ports.agentForkGateway!.fork(request), proof = await ports.agentForkGateway!.status(request);
  expect(proof.status.type).toBe("committed");
  if (proof.status.type !== "committed" || before.status.type !== "noSqlReceipt") throw Error("Invalid fixture state");
  expect(proof.status.proof.epoch).toBe(before.status.epoch);
  await ports.agentArchiveGateway!.setArchived({ operationId: crypto.randomUUID(), id: child.conversation.id, archived: true, expectedWatermark: child.conversation.durableCount });
  await ports.agentDeletionGateway!.delete({ schemaVersion: 1, requestId: crypto.randomUUID(), conversationId: child.conversation.id, binding,
    expectedTitle: child.conversation.title, expectedDurableCount: child.conversation.durableCount, force: true });
  await writing.archiveManuscript(manuscript.id); await writing.deleteManuscript(manuscript.id);
  const reopened = open(), fresh = reopened.gateways(), stored = snapshot(storage);
  const runtime = vi.spyOn(reopened, "backend").mockImplementation(() => { throw Error("must not open runtime"); });
  try {
    expect(await fresh.agentForkGateway!.status(request)).toEqual(proof);
    await expect(fresh.agentForkGateway!.status({ ...request, throughTurnId: crypto.randomUUID() })).rejects.toMatchObject({ code: "storage", reason: { code: "DATA_SQL_OPERATION_CONFLICT" } });
    expect(runtime).not.toHaveBeenCalled(); expect(snapshot(storage)).toEqual(stored);
  } finally { runtime.mockRestore(); }
});
