// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { MockWritingAgents } from "./MockWritingAgents";
import { MockIndependentAgents } from "./MockIndependentAgents";
import { createMockWritingGateway } from "./mockWritingGateway";
import { createBrowserWritingCheckpointGateway } from "../../../../../../plugins/writing_projects/src/features/writing/gateway/browserWritingCheckpointGateway";
import { createPreviewControls } from "../../../core/runtime/previewControls";
import { createTestNamespacedStorage } from "../../../core/runtime/testing/createTestNamespacedStorage";
import { registerWritingRecords } from "../../../core/features/writing-storage/browserWritingRecords";
import { memoryWritingRecords, memoryWritingState } from "../testing/memoryWritingRecords";
import { createMockConversationCatalog } from "./MockConversationCatalog";
const live: MockWritingAgents[] = [];
afterEach(() => { for (const owner of live.splice(0)) owner.dispose(); });
function stored(storage: Storage) { return new Map(Array.from({length: storage.length}, (_, i) => { const key = storage.key(i)!; return [key, storage.getItem(key)]; })); }
async function fixture() {
  const storage = createTestNamespacedStorage(localStorage, `${crypto.randomUUID()}:`);
  registerWritingRecords(storage, memoryWritingRecords(memoryWritingState()));
  const writing = createMockWritingGateway(storage), manuscript = await writing.createManuscript({title: "Synthetic"});
  const controls = createPreviewControls(storage, () => undefined); controls.configure({delayMs: 0, nextFailure: null});
  const open = () => { const owner = new MockWritingAgents(storage, writing, createBrowserWritingCheckpointGateway(storage), controls); live.push(owner); return owner; };
  const owner = open(), ports = owner.gateways(), id = crypto.randomUUID(), binding = {kind: "manuscript" as const, resourceId: manuscript.id};
  const creation = {operationId: crypto.randomUUID(), id, binding};
  await ports.agentCreationGateway!.createBound(creation);
  await ports.agentArchiveGateway!.setArchived({operationId: crypto.randomUUID(), id, archived: true, expectedWatermark: 0});
  const request = {schemaVersion: 1 as const, requestId: crypto.randomUUID(), conversationId: id, binding,
    expectedTitle: "新对话", expectedDurableCount: 0, force: true};
  return {storage, writing, manuscript, controls, owner, ports, open, request, creation};
}
it("keeps exact deletion evidence after cold reopen and resource removal without opening a runtime or writing storage", async () => {
  const f = await fixture(), before = await f.ports.agentDeletionGateway!.status(f.request);
  if (before.status.type !== "noSqlReceipt") throw Error("Invalid fixture");
  await f.ports.agentDeletionGateway!.delete(f.request, {expectedEpoch: before.status.epoch});
  const proof = await f.ports.agentDeletionGateway!.status(f.request); expect(proof.status.type).toBe("committed");
  await f.writing.archiveManuscript(f.manuscript.id); await f.writing.deleteManuscript(f.manuscript.id);
  const reopened = f.open(), ports = reopened.gateways(), saved = stored(f.storage);
  const backend = vi.spyOn(reopened, "backend").mockImplementation(() => { throw Error("must not open runtime"); });
  const listing = vi.spyOn(f.writing, "listManuscripts").mockRejectedValue(Error("must not read resources"));
  try {
    expect(await ports.agentDeletionGateway!.status(f.request)).toEqual(proof);
    expect(await ports.agentDeletionGateway!.delete(f.request, {expectedEpoch: before.status.epoch})).toMatchObject({requestId: f.request.requestId});
    for (const patch of [{expectedTitle: "Changed"}, {force: false}, {expectedDurableCount: 1}, {conversationId: crypto.randomUUID()}]) {
      await expect(ports.agentDeletionGateway!.status({...f.request, ...patch})).rejects.toMatchObject({reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
    }
    await expect(ports.agentDeletionGateway!.delete(f.request, {expectedEpoch: crypto.randomUUID()}))
      .rejects.toMatchObject({reason: {reason: {phase: "delete-epoch", safeCause: "data_epoch_changed"}}});
    expect(backend).not.toHaveBeenCalled(); expect(listing).not.toHaveBeenCalled(); expect(stored(f.storage)).toEqual(saved);
  } finally { backend.mockRestore(); listing.mockRestore(); }
});
it("refuses foreign epoch and operation families before changing mock data", async () => {
  const f = await fixture(), saved = stored(f.storage);
  const backend = vi.spyOn(f.owner, "backend");
  await expect(f.ports.agentDeletionGateway!.delete(f.request, {expectedEpoch: crypto.randomUUID()}))
    .rejects.toMatchObject({reason: {reason: {phase: "delete-epoch"}}});
  await expect(f.ports.agentDeletionGateway!.delete({...f.request, requestId: f.creation.operationId}))
    .rejects.toMatchObject({reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
  expect(backend).not.toHaveBeenCalled(); expect(stored(f.storage)).toEqual(saved);
});
it("routes independent deletion proofs after conversation removal and reopen without replaying mutation", async () => {
  const f = await fixture(), independent = new MockIndependentAgents(f.storage, f.controls);
  const id = crypto.randomUUID();
  try {
    await independent.gateway.sessions.create(id);
    await independent.archives.setArchived({operationId: crypto.randomUUID(), id, archived: true, expectedWatermark: 0});
    const request = {...f.request, requestId: crypto.randomUUID(), conversationId: id, binding: null};
    const before = await independent.deletions.status(request);
    if (before.status.type !== "noSqlReceipt") throw Error("Invalid fixture");
    await independent.deletions.delete(request, {expectedEpoch: before.status.epoch});
    const proof = await independent.deletions.status(request); independent.dispose();
    const reopened = new MockIndependentAgents(f.storage, f.controls), saved = stored(f.storage);
    const unified = createMockConversationCatalog(reopened, f.owner, f.ports);
    try {
      expect(await unified.agentDeletionGateway!.status(request)).toEqual(proof);
      await expect(unified.agentDeletionGateway!.delete(request, {expectedEpoch: crypto.randomUUID()})).rejects.toMatchObject({reason: {reason: {phase: "delete-epoch"}}});
      expect(stored(f.storage)).toEqual(saved); expect(reopened.has(id)).toBe(false);
    } finally { reopened.dispose(); }
  } finally { independent.dispose(); }
});
