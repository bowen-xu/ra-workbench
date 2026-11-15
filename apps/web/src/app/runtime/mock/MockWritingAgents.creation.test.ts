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
  const owner = open(), request = {operationId: crypto.randomUUID(), id: crypto.randomUUID(), binding: {kind: "manuscript" as const, resourceId: one.id}};
  return {storage, writing, one, two, owner, request, open, controls};
}
function stored(storage: Storage) { return new Map(Array.from({length: storage.length}, (_, i) => { const key = storage.key(i)!; return [key, storage.getItem(key)]; })); }

it("globally fences C from another manuscript, including concurrent creation and changed original operation", async () => {
  const f = await fixture(), gateway = f.owner.gateways().agentCreationGateway!;
  const [first, second] = await Promise.allSettled([gateway.createBound(f.request), gateway.createBound({...f.request,
    operationId: crypto.randomUUID(), binding: {...f.request.binding, resourceId: f.two.id}})]);
  expect(first.status).toBe("fulfilled"); expect(second).toMatchObject({status: "rejected", reason: "conflict"});
  await expect(gateway.createBound({...f.request, binding: {...f.request.binding, resourceId: f.two.id}})).rejects.toMatchObject({reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
  expect((await f.owner.gateways().agentMetadataGateway!.read({id: f.request.id, binding: f.request.binding})).title).toBe("新对话");
});
it("retains proof after resource deletion and refresh, with metadata-only status and no backend initialization", async () => {
  const f = await fixture(), gateway = f.owner.gateways().agentCreationGateway!, proof = await gateway.createBound(f.request);
  await f.writing.archiveManuscript(f.one.id); await f.writing.deleteManuscript(f.one.id);
  await expect(gateway.createBound(f.request)).rejects.toBe("notFound");
  const reopened = f.open(), ports = reopened.gateways(), before = stored(f.storage);
  const listing = vi.spyOn(f.writing, "listManuscripts").mockRejectedValue(new Error("must not read live resources"));
  expect((await ports.agentCreationGateway!.createBoundStatus(f.request)).status).toEqual({type: "committed", proof: {...proof, replayed: true}});
  expect((await ports.agentCreationGateway!.createBoundStatus({...f.request, operationId: crypto.randomUUID(), binding: {...f.request.binding, resourceId: "missing"}})).status.type).toBe("noSqlReceipt");
  expect(listing).not.toHaveBeenCalled(); expect(stored(f.storage)).toEqual(before); listing.mockRestore();
});
it("rejects unbound adoption and shared rename/create operation collisions in either mock host", async () => {
  const f = await fixture(), independent = new MockIndependentAgents(f.storage, f.controls);
  f.owner.unboundConversationExists = id => independent.has(id);
  f.owner.foreignOperationExists = id => independent.hasRenameOperation(id);
  independent.foreignOperationExists = id => f.owner.hasOperation(id);
  const ports = f.owner.gateways(), unified = createMockConversationCatalog(independent, f.owner, ports);
  try {
    const unbound = crypto.randomUUID(); await independent.gateway.sessions.create(unbound);
    await expect(ports.agentCreationGateway!.createBound({...f.request, id: unbound})).rejects.toBe("conflict");
    await ports.agentCreationGateway!.createBound(f.request);
    await expect(unified.agentCatalogGateway!.rename({operationId: f.request.operationId, id: unbound, expectedTitle: "新对话", title: "Other"})).rejects.toMatchObject({reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
    const op = crypto.randomUUID(), row = await independent.metadata.read({id: unbound});
    await unified.agentCatalogGateway!.rename({operationId: op, id: unbound, expectedTitle: row.title, title: "Other"});
    await expect(ports.agentCreationGateway!.createBound({...f.request, operationId: op, id: crypto.randomUUID()})).rejects.toMatchObject({reason: {code: "DATA_SQL_OPERATION_CONFLICT"}});
  } finally { independent.dispose(); }
});
it("replays historical creation after an explicit directory move without moving C back", async () => {
  const f = await fixture(), ports = f.owner.gateways(), proof = await ports.agentCreationGateway!.createBound(f.request);
  const independent = new MockIndependentAgents(f.storage, f.controls);
  try {
    await f.owner.moveConversation(f.request.id, f.two.id, independent);
    expect(await ports.agentCreationGateway!.createBound(f.request)).toEqual({...proof, replayed: true});
    expect((await f.owner.contextDirectory(f.request.id))?.manuscriptId).toBe(f.two.id);
    await expect(ports.agentCreationGateway!.createBound({...f.request, operationId: crypto.randomUUID()})).rejects.toBe("conflict");
    expect((await ports.agentCreationGateway!.createBoundStatus(f.request)).status.type).toBe("committed");
  } finally { independent.dispose(); }
});
