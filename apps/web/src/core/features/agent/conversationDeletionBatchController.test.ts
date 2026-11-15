import { expect, it, vi } from "vitest";
import type { AgentDeletionGateway } from "@research-assistant/data/agent";
import type { AgentDeletionRequest } from "@research-assistant/core/agent";
import { createMockAgentRecoveryStorage } from "../../../app/runtime/mock/MockAgentRecoveryStorage";
import { registerStoragePersistence } from "../../runtime/storagePersistence";
import { MockDeletionReceipts } from "./MockDeletionReceipts";
import { canRetainUnsentBatchMembers, ConversationDeletionBatchController } from "./conversationDeletionBatchController";
import { discoverConversationDeletionBatches, readConversationDeletionBatch, saveConversationDeletionBatch } from "./conversationDeletionBatchPending";
import { readConversationDeletion } from "./conversationDeletionPending";
import { ConversationDeletionController, resetConversationDeletionOwnersForTests } from "./conversationDeletionController";
import type { ConversationOperationLock } from "./conversationOperationLock";
import { deletionBatchAction, deletionBatchMessage } from "./conversationDeletionBatchMessage";

const uuid = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const binding = {kind: "manuscript" as const, resourceId: "synthetic"};
const requests: readonly AgentDeletionRequest[] = [1, 2, 3].map(n => ({schemaVersion: 1, requestId: uuid(n + 10),
  conversationId: uuid(n), binding, expectedTitle: `Archive ${n}`, expectedDurableCount: n, force: true}));
function fixture() {
  let epoch = uuid(99);
  const storage = createMockAgentRecoveryStorage("synthetic-batch"), receipts = new MockDeletionReceipts(() => epoch);
  const held = new Set<string>(), lock: ConversationOperationLock = {async run(name, action) {
    if (held.has(name)) throw Error("DELETION_BATCH_LOCK_BUSY");
    held.add(name); try {return await action();} finally {held.delete(name);}
  }};
  const mutation = vi.fn(), cleanup = vi.fn();
  const status = vi.fn<AgentDeletionGateway["status"]>(async request => receipts.status(request));
  const remove = vi.fn<AgentDeletionGateway["delete"]>(async (request, options) => {
    receipts.validateWrite(request, options); receipts.apply(request, () => mutation(request.conversationId));
    return {schemaVersion: 1, requestId: request.requestId, conversationId: request.conversationId};
  });
  const gateway = {delete: remove, status}, ports = {deletions: gateway, cleanup, lock};
  const open = () => new ConversationDeletionBatchController(binding, false, ports, storage);
  const cold = (owner: ConversationDeletionBatchController) => {owner.dispose(); resetConversationDeletionOwnersForTests(storage, gateway); return open();};
  return {storage, receipts, mutation, cleanup, status, remove, gateway, ports, open, cold, setEpoch: (value: string) => {epoch = value;}};
}
function persistence(storage: Storage) {
  const flush = vi.fn<() => Promise<void>>(async () => undefined), retry = vi.fn(async () => undefined);
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush, retry, assertCurrent: async () => undefined, reset: async () => undefined, subscribe: () => () => undefined, close: () => undefined});
  return {flush, retry};
}
it("saves the full frozen batch before readonly IPC and reserves every original member before the first mutation", async () => {
  const f = fixture(), owner = f.open(), remove = f.remove.getMockImplementation()!;
  f.status.mockImplementation(async request => {
    expect(discoverConversationDeletionBatches(f.storage)[0]?.requests).toEqual(requests); return f.receipts.status(request);
  });
  f.remove.mockImplementation(async (request, options) => {
    expect(discoverConversationDeletionBatches(f.storage)[0]?.phase).toBe("executing");
    for (const original of requests) {
      const record = readConversationDeletion(f.storage, original.conversationId);
      if (!f.receipts.has(original.requestId)) expect(record).toMatchObject({intent: original, epoch: uuid(99)});
    }
    expect(options).toEqual({expectedEpoch: uuid(99)}); return remove(request, options);
  });
  expect(await owner.start(requests)).toBe(true);
  expect(f.remove.mock.calls.map(([request]) => request)).toEqual(requests);
  expect(f.cleanup).toHaveBeenCalledTimes(3); expect(f.storage.length).toBe(0); expect(owner.getSnapshot().phase).toBe("ready");
});
it("does not send even readonly IPC before the whole batch is reliable, and local repair never deletes", async () => {
  const f = fixture(), port = persistence(f.storage), owner = f.open(); port.flush.mockRejectedValueOnce(Error("disk"));
  expect(await owner.start(requests)).toBe(false); expect(f.status).not.toHaveBeenCalled(); expect(f.remove).not.toHaveBeenCalled();
  const original = owner.getSnapshot().batch!;
  expect(await owner.retryLocal()).toBe(false); expect(f.remove).not.toHaveBeenCalled();
  expect(owner.getSnapshot().batch?.batchId).toBe(original.batchId);
  expect(await owner.continueOriginal()).toBe(true); expect(f.remove.mock.calls.map(([request]) => request)).toEqual(original.requests);
});
it("finds committed members after a cold lost ACK, performs checks only and finishes without deleting twice", async () => {
  const f = fixture(), owner = f.open(), remove = f.remove.getMockImplementation()!;
  f.remove.mockImplementation(async (...args) => {await remove(...args); throw "unavailable";});
  expect(await owner.start(requests)).toBe(false); const batch = owner.getSnapshot().batch!;
  expect(f.mutation).toHaveBeenCalledTimes(3);
  const reopened = f.cold(owner); expect(reopened.getSnapshot().batch).toEqual(batch);
  expect(f.remove).toHaveBeenCalledTimes(3); await reopened.check(); expect(f.remove).toHaveBeenCalledTimes(3);
  expect(f.cleanup).not.toHaveBeenCalled(); expect(await reopened.finish()).toBe(true);
  expect(f.remove).toHaveBeenCalledTimes(3); expect(f.cleanup).toHaveBeenCalledTimes(3); expect(f.storage.length).toBe(0);
});
it("retains untouched members and exact requests when a later local failure interrupts execution, then resumes the original set", async () => {
  const f = fixture(), port = persistence(f.storage), owner = f.open(); let fail = true;
  port.flush.mockImplementation(async () => {
    if (fail && readConversationDeletion(f.storage, requests[1]!.conversationId)?.stage === "unknown") throw Error("second save failed");
  });
  expect(await owner.start(requests)).toBe(false); const original = owner.getSnapshot().batch!;
  expect(f.remove).toHaveBeenCalledOnce(); expect(original.requests).toEqual(requests);
  expect(readConversationDeletion(f.storage, requests[2]!.conversationId)?.stage).toBe("pending");
  fail = false; const reopened = f.cold(owner); await reopened.check();
  expect(f.remove).toHaveBeenCalledOnce(); expect(await reopened.continueOriginal()).toBe(true);
  expect(f.remove.mock.calls.map(([request]) => request)).toEqual(requests); expect(f.mutation).toHaveBeenCalledTimes(3);
});
it("records finishing before releasing known refusals and never reauthorizes them after final local removal fails", async () => {
  const f = fixture(), owner = f.open(), remove = f.remove.getMockImplementation()!;
  f.remove.mockImplementation(async (...args) => {if (args[0].conversationId === requests[1]!.conversationId) throw "notArchived"; return remove(...args);});
  expect(await owner.start(requests)).toBe(false); expect(f.cleanup).toHaveBeenCalledTimes(2);
  const removeItem = f.storage.removeItem.bind(f.storage);
  vi.spyOn(f.storage, "removeItem").mockImplementationOnce(name => {
    expect(discoverConversationDeletionBatches(f.storage)[0]?.phase).toBe("finishing"); removeItem(name);
  }).mockImplementationOnce(() => {throw Error("batch removal failed");});
  expect(await owner.finish()).toBe(false);
  const saved = discoverConversationDeletionBatches(f.storage)[0]!;
  expect(saved).toMatchObject({phase: "finishing", retained: [requests[1]!.conversationId]});
  vi.restoreAllMocks(); f.remove.mockImplementation(remove); const reopened = f.cold(owner);
  await reopened.check(); expect(await reopened.continueOriginal()).toBe(true);
  expect(f.remove).toHaveBeenCalledTimes(3); expect(f.mutation).toHaveBeenCalledTimes(2); expect(f.storage.length).toBe(0);
});
it("keeps every member fenced to the first saved epoch across environment changes", async () => {
  const f = fixture(), owner = f.open(), remove = f.remove.getMockImplementation()!;
  f.remove.mockImplementationOnce(async (...args) => {const result = await remove(...args); f.setEpoch(uuid(98)); return result;});
  expect(await owner.start(requests)).toBe(false); expect(f.remove).toHaveBeenCalledOnce();
  expect(owner.getSnapshot().batch?.epoch).toBe(uuid(99));
  for (const request of requests.slice(1)) expect(owner.getSnapshot().items[request.conversationId]?.cause).toMatchObject({message: "DELETION_RECOVERY_EPOCH_CHANGED"});
  const reopened = f.cold(owner); await reopened.check(); expect(await reopened.continueOriginal()).toBe(false);
  expect(f.remove).toHaveBeenCalledOnce(); expect(discoverConversationDeletionBatches(f.storage)[0]?.requests).toEqual(requests);
});
it("excludes another window for the full batch and prevents single or overlapping batch allocation into its reserved set", async () => {
  const f = fixture(), owner = f.open(); let release!: () => void;
  f.remove.mockImplementationOnce(() => new Promise(resolve => {release = () => resolve({schemaVersion: 1, requestId: requests[0]!.requestId, conversationId: requests[0]!.conversationId});}));
  const pending = owner.start(requests); await vi.waitFor(() => expect(f.remove).toHaveBeenCalledOnce());
  const other = f.open(); expect(await other.continueOriginal()).toBe(false);
  expect(other.getSnapshot().issue).toMatchObject({message: "DELETION_BATCH_LOCK_BUSY"});
  const single = new ConversationDeletionController(requests[2]!.conversationId, f.ports, f.storage);
  expect(await single.submit({id: requests[2]!.conversationId, title: "Different", archived: true, durableCount: 0}, binding)).toBe(false);
  expect(single.getSnapshot().record?.intent).toEqual(requests[2]);
  const batch = owner.getSnapshot().batch!;
  expect(() => saveConversationDeletionBatch(f.storage, {...batch, batchId: uuid(90), binding: null,
    requests: requests.map(request => ({...request, requestId: uuid(Number(request.expectedDurableCount) + 30), binding: null}))})).toThrow("DELETION_BATCH_CONFLICT");
  release(); await pending; other.dispose();
});
it("rejects a rewritten frozen set, epoch or finishing authorization while preserving the original bytes", () => {
  const f = fixture(), batch = {schemaVersion: 1 as const, batchId: uuid(80), binding, internal: false,
    epoch: uuid(99), requests, phase: "authorized" as const, retained: []};
  saveConversationDeletionBatch(f.storage, batch);
  expect(() => saveConversationDeletionBatch(f.storage, {...batch, requests: requests.slice(1)})).toThrow("DELETION_BATCH_CONFLICT");
  expect(() => saveConversationDeletionBatch(f.storage, {...batch, epoch: uuid(98)})).toThrow("DELETION_BATCH_CONFLICT");
  const finishing = {...batch, phase: "finishing" as const, retained: [requests[1]!.conversationId]}; saveConversationDeletionBatch(f.storage, finishing);
  expect(() => saveConversationDeletionBatch(f.storage, batch)).toThrow("DELETION_BATCH_CONFLICT");
  expect(readConversationDeletionBatch(f.storage, batch.batchId)).toEqual(finishing);
});
it("keeps the first readonly failure as an original-request diagnostic, distinct from local save failure", async () => {
  const f = fixture(), owner = f.open();
  f.status.mockRejectedValueOnce({code: "agent_preview_operation_unconfirmed", status: 503, message: "private response"});
  expect(await owner.start(requests)).toBe(false); expect(f.remove).not.toHaveBeenCalled();
  expect(owner.getSnapshot().issue).toBeNull(); expect(deletionBatchAction(owner.getSnapshot())).toBe("check");
  const message = deletionBatchMessage(owner.getSnapshot());
  expect(message).toContain(requests[0]!.requestId); expect(message).toContain("HTTP 503"); expect(message).toContain("agent_preview_operation_unconfirmed");
  expect(message).not.toContain("private"); expect(message).not.toContain("DELETION_BATCH_RECOVERY_UNAVAILABLE");
  await owner.check(); expect(f.remove).not.toHaveBeenCalled(); expect(await owner.continueOriginal()).toBe(true);
});
it("cannot write a member until the original epoch and whole-batch execution gate are both reliable", async () => {
  const f = fixture(), batch = {schemaVersion: 1 as const, batchId: uuid(80), binding, internal: false,
    epoch: null, requests, phase: "authorized" as const, retained: []};
  saveConversationDeletionBatch(f.storage, batch);
  const member = new ConversationDeletionController(requests[0]!.conversationId, f.ports, f.storage);
  expect(await member.prepareBatchMember(batch, requests[0]!)).toBe(true); await member.check();
  expect(member.getSnapshot().record?.epoch).toBe(uuid(99));
  expect(await member.retryOriginal()).toBe(false); expect(f.remove).not.toHaveBeenCalled();
  expect(member.getSnapshot().localIssue).toMatchObject({message: "DELETION_RECOVERY_BATCH_RESERVED"});
  saveConversationDeletionBatch(f.storage, {...batch, epoch: uuid(99)}); await member.retryLocal();
  expect(await member.retryOriginal()).toBe(false); expect(f.remove).not.toHaveBeenCalled();
});
it("never adopts or repairs another operation's member owner to advance a batch", async () => {
  const f = fixture(), single = new ConversationDeletionController(requests[0]!.conversationId, f.ports, f.storage);
  f.remove.mockRejectedValueOnce("unavailable");
  await single.submit({id: requests[0]!.conversationId, title: "Other original", archived: true, durableCount: 1}, binding);
  const original = single.getSnapshot().record!, owner = f.open(), queries = f.status.mock.calls.length;
  expect(await owner.start(requests)).toBe(false); expect(owner.getSnapshot().issue).toMatchObject({message: "DELETION_BATCH_CONFLICT"});
  expect(await owner.retryLocal()).toBe(false); expect(f.status).toHaveBeenCalledTimes(queries);
  expect(f.remove).toHaveBeenCalledOnce(); expect(readConversationDeletion(f.storage, requests[0]!.conversationId)).toEqual(original);
  expect(single.getSnapshot().localIssue).toBeNull();
  await single.check(); expect(await single.retryOriginal()).toBe(true);
  expect(f.remove.mock.calls.at(-1)![0]).toEqual(original.intent);
});
it("can explicitly retain every unsent item after a readonly failure, without requiring another backend query", async () => {
  const f = fixture(), owner = f.open(); f.status.mockRejectedValueOnce("unavailable");
  await owner.start(requests); expect(owner.getSnapshot().batch?.epoch).toBeNull();
  expect(canRetainUnsentBatchMembers(owner.getSnapshot())).toBe(true);
  expect(await owner.retainUnsent()).toBe(true); expect(f.status).toHaveBeenCalledOnce(); expect(f.remove).not.toHaveBeenCalled();
  expect(owner.getSnapshot().batch?.retained).toEqual(requests.map(request => request.conversationId)); expect(f.storage.length).toBe(0);
});
it("does not allow retaining an unknown/partial member even when other members are still unsent", async () => {
  const f = fixture(), owner = f.open(), port = persistence(f.storage);
  f.remove.mockRejectedValueOnce({code: "storage", reason: {code: "DATA_OPERATION_PARTIAL"}});
  port.flush.mockImplementation(async () => {if (f.remove.mock.calls.length && readConversationDeletion(f.storage, requests[1]!.conversationId)?.stage === "unknown") throw Error("keep pending");});
  await owner.start(requests); expect(canRetainUnsentBatchMembers(owner.getSnapshot())).toBe(false);
  expect(await owner.retainUnsent()).toBe(false); expect(discoverConversationDeletionBatches(f.storage)[0]?.phase).toBe("executing");
});
it("serializes whole-batch admission across a bound and global actor until the original manifest is reliable", async () => {
  const f = fixture(), port = persistence(f.storage), owner = f.open(); let release!: () => void;
  port.flush.mockImplementationOnce(() => new Promise<void>(resolve => {release = resolve;}));
  const active = owner.start(requests); await vi.waitFor(() => expect(port.flush).toHaveBeenCalledOnce());
  const other = new ConversationDeletionBatchController(null, false, f.ports, f.storage);
  expect(await other.start(requests.map(request => ({...request, binding: null, requestId: uuid(request.expectedDurableCount + 40)})))).toBe(false);
  expect(other.getSnapshot().issue).toMatchObject({message: "DELETION_BATCH_LOCK_BUSY"});
  expect(discoverConversationDeletionBatches(f.storage)).toHaveLength(1); expect(f.status).not.toHaveBeenCalled(); expect(f.remove).not.toHaveBeenCalled();
  release(); expect(await active).toBe(true);
});
it("lets a user retain explicitly rejected busy members without waiting for or restarting their running tasks", async () => {
  const f = fixture(), owner = f.open(); f.remove.mockRejectedValue("busy");
  await owner.start(requests); expect(canRetainUnsentBatchMembers(owner.getSnapshot())).toBe(true);
  expect(await owner.retainUnsent()).toBe(true); expect(f.remove).toHaveBeenCalledTimes(3);
  expect(f.mutation).not.toHaveBeenCalled(); expect(f.cleanup).not.toHaveBeenCalled(); expect(f.storage.length).toBe(0);
});
it("does not turn missing members of an executable cold batch into unsent authority when legacy proof is unavailable", async () => {
  const f = fixture(), owner = f.open();
  f.status.mockImplementation(async request => {
    if (f.receipts.has(requests[0]!.requestId) && request.conversationId !== requests[0]!.conversationId) throw "unavailable";
    return f.receipts.status(request);
  });
  await owner.start(requests); expect(f.remove).toHaveBeenCalledOnce();
  expect(owner.getSnapshot().batch?.phase).toBe("executing"); expect(readConversationDeletion(f.storage, requests[0]!.conversationId)).toBeNull();
  f.receipts.restore(undefined, new Map([[requests[0]!.requestId, JSON.stringify(requests[0])]]));
  f.status.mockImplementation(async request => f.receipts.status(request));
  const reopened = f.cold(owner); await reopened.check();
  expect(reopened.getSnapshot().items[requests[0]!.conversationId]?.record?.stage).toBe("unknown");
  expect(canRetainUnsentBatchMembers(reopened.getSnapshot())).toBe(false); expect(await reopened.retainUnsent()).toBe(false);
  expect(f.remove).toHaveBeenCalledOnce(); expect(discoverConversationDeletionBatches(f.storage)[0]?.phase).toBe("executing");
});
