// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import { MockWritingAgents } from "./MockWritingAgents";
import { MockIndependentAgents } from "./MockIndependentAgents";
import { createMockWritingGateway } from "./mockWritingGateway";
import { createBrowserWritingCheckpointGateway } from "../../../../../../plugins/writing_projects/src/features/writing/gateway/browserWritingCheckpointGateway";
import { createPreviewControls } from "../../../core/runtime/previewControls";
import { createTestNamespacedStorage } from "../../../core/runtime/testing/createTestNamespacedStorage";
import { createMockConversationCatalog } from "./MockConversationCatalog";
import { registerWritingRecords } from "../../../core/features/writing-storage/browserWritingRecords";
import { memoryWritingRecords, memoryWritingState } from "../testing/memoryWritingRecords";

const live: MockWritingAgents[] = [];
afterEach(() => { for (const owner of live.splice(0)) owner.dispose(); });
async function fixture() {
  const storage = createTestNamespacedStorage(localStorage, `${crypto.randomUUID()}:`);
  registerWritingRecords(storage, memoryWritingRecords(memoryWritingState()));
  const writing = createMockWritingGateway(storage);
  const one = await writing.createManuscript({title: "One"}), two = await writing.createManuscript({title: "Two"});
  const controls = createPreviewControls(storage, () => undefined); controls.configure({delayMs: 0, nextFailure: null});
  const open = () => {
    const owner = new MockWritingAgents(storage, writing, createBrowserWritingCheckpointGateway(storage), controls);
    live.push(owner); return owner;
  };
  const owner = open(), ports = owner.gateways(), id = crypto.randomUUID();
  const creation = {operationId: crypto.randomUUID(), id, binding: {kind: "manuscript" as const, resourceId: one.id}};
  await ports.agentCreationGateway!.createBound(creation);
  const request = {operationId: crypto.randomUUID(), id, archived: true, expectedWatermark: 0};
  return {storage, writing, one, two, owner, ports, creation, request, open, controls};
}
function stored(storage: Storage) { return new Map(Array.from({length: storage.length}, (_, i) => { const key = storage.key(i)!; return [key, storage.getItem(key)]; })); }

it("replays the original archive across directory move and refresh without restoring the old flag", async () => {
  const f = await fixture(), proof = await f.ports.agentArchiveGateway!.setArchived(f.request);
  await f.ports.agentArchiveGateway!.setArchived({...f.request, operationId: crypto.randomUUID(), archived: false, expectedWatermark: null});
  const independent = new MockIndependentAgents(f.storage, f.controls);
  try {
    await f.owner.moveConversation(f.request.id, f.two.id, independent);
    const reopened = f.open().gateways(), before = stored(f.storage);
    expect(await reopened.agentArchiveGateway!.setArchived(f.request)).toEqual({...proof, replayed: true});
    expect((await reopened.agentArchiveGateway!.archiveStatus(f.request)).status.type).toBe("committed");
    expect(stored(f.storage)).toEqual(before);
    expect((await reopened.agentMetadataGateway!.read({id: f.request.id})).archived).toBe(false);
  } finally { independent.dispose(); }
});

it("reads historical status without loading deleted resources and never resurrects a deleted conversation", async () => {
  const f = await fixture(), proof = await f.ports.agentArchiveGateway!.setArchived(f.request);
  await f.ports.agentDeletionGateway!.delete({schemaVersion: 1, requestId: crypto.randomUUID(), conversationId: f.request.id,
    binding: f.creation.binding, expectedTitle: "新对话", expectedDurableCount: 0, force: true});
  await f.writing.archiveManuscript(f.one.id); await f.writing.deleteManuscript(f.one.id);
  const reopened = f.open().gateways(), before = stored(f.storage);
  const listing = vi.spyOn(f.writing, "listManuscripts").mockRejectedValue(new Error("must not read live resources"));
  try {
    expect(await reopened.agentArchiveGateway!.setArchived(f.request)).toEqual({...proof, replayed: true});
    expect((await reopened.agentArchiveGateway!.archiveStatus(f.request)).status).toEqual({type: "committed", proof: {...proof, replayed: true}});
    expect((await reopened.agentArchiveGateway!.archiveStatus({...f.request, operationId: crypto.randomUUID()})).status.type).toBe("noSqlReceipt");
    expect(listing).not.toHaveBeenCalled(); expect(stored(f.storage)).toEqual(before);
  } finally { listing.mockRestore(); }
});

it("shares operation identities across archive, rename and creation on bound and independent hosts", async () => {
  const f = await fixture(), independent = new MockIndependentAgents(f.storage, f.controls);
  f.owner.foreignOperationExists = id => independent.hasRenameOperation(id) || independent.hasArchiveOperation(id);
  independent.foreignOperationExists = id => f.owner.hasOperation(id);
  const unified = createMockConversationCatalog(independent, f.owner, f.ports);
  try {
    const unbound = crypto.randomUUID(); await independent.gateway.sessions.create(unbound);
    await expect(unified.agentArchiveGateway!.setArchived({...f.request, operationId: f.creation.operationId}))
      .rejects.toMatchObject({reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
    await unified.agentArchiveGateway!.setArchived(f.request);
    await expect(unified.agentArchiveGateway!.setArchived({...f.request, id: unbound}))
      .rejects.toMatchObject({reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
    const other = {...f.request, id: unbound, operationId: crypto.randomUUID()};
    await unified.agentArchiveGateway!.setArchived(other);
    await expect(f.ports.agentCreationGateway!.createBound({...f.creation, operationId: other.operationId, id: crypto.randomUUID()}))
      .rejects.toMatchObject({reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
    await expect(unified.agentCatalogGateway!.rename({operationId: f.request.operationId, id: unbound, expectedTitle: "新对话", title: "别名"}))
      .rejects.toMatchObject({reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
    await independent.deletions.delete({schemaVersion: 1, requestId: crypto.randomUUID(), conversationId: unbound,
      binding: null, expectedTitle: "新对话", expectedDurableCount: 0, force: true});
    expect((await unified.agentArchiveGateway!.archiveStatus(other)).status.type).toBe("committed");
    expect((await unified.agentArchiveGateway!.setArchived(other)).replayed).toBe(true);
    expect(independent.has(unbound)).toBe(false);
  } finally { independent.dispose(); }
});

it("offers the same batch gateway for mixed archived and retained preview conversations", async () => {
  const f = await fixture(), independent = new MockIndependentAgents(f.storage, f.controls);
  const unified = createMockConversationCatalog(independent, f.owner, f.ports);
  try {
    await unified.agentArchiveGateway!.setArchived(f.request);
    const unbound = crypto.randomUUID(); await independent.gateway.sessions.create(unbound);
    const first = {schemaVersion: 1 as const, requestId: crypto.randomUUID(), conversationId: f.request.id,
      binding: f.creation.binding, expectedTitle: "新对话", expectedDurableCount: 0, force: true};
    const second = {...first, requestId: crypto.randomUUID(), conversationId: unbound, binding: null};
    const response = await unified.agentDeletionBatchGateway!.deleteBatch([first, second]);
    expect(response.results).toMatchObject([
      {requestId: first.requestId, conversationId: first.conversationId, status: "deleted"},
      {requestId: second.requestId, conversationId: second.conversationId, status: "failed", error: "notArchived"},
    ]);
    expect(independent.has(unbound)).toBe(true);
  } finally {independent.dispose();}
});

it("publishes one batch across bound and independent preview stores without calling individual delete gateways", async () => {
  const f = await fixture(), independent = new MockIndependentAgents(f.storage, f.controls);
  const unified = createMockConversationCatalog(independent, f.owner, f.ports);
  try {
    const unbound = crypto.randomUUID(); await independent.gateway.sessions.create(unbound);
    await unified.agentArchiveGateway!.setArchived(f.request);
    await unified.agentArchiveGateway!.setArchived({operationId: crypto.randomUUID(), id: unbound, archived: true, expectedWatermark: 0});
    const first = {schemaVersion: 1 as const, requestId: crypto.randomUUID(), conversationId: f.request.id,
      binding: f.creation.binding, expectedTitle: "新对话", expectedDurableCount: 0, force: true};
    const second = {...first, requestId: crypto.randomUUID(), conversationId: unbound, binding: null};
    const singleIndependent = vi.spyOn(independent.deletions, "delete");
    const singleBound = vi.spyOn(f.ports.agentDeletionGateway!, "delete");
    const response = await unified.agentDeletionBatchGateway!.deleteBatch([first, second]);
    expect(response.results.map(result => result.status)).toEqual(["deleted", "deleted"]);
    expect(singleIndependent).not.toHaveBeenCalled(); expect(singleBound).not.toHaveBeenCalled();
    expect(independent.has(unbound)).toBe(false);
    await expect(unified.agentMetadataGateway!.read({id: first.conversationId})).rejects.toBe("notFound");
  } finally {independent.dispose();}
});
