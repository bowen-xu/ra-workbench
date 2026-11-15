import { expect, it, vi } from "vitest";
import type { AgentDeletionGateway } from "@research-assistant/data/agent";
import type { AgentConversationSummary, AgentDeletionRequest } from "@research-assistant/core/agent";
import { registerStoragePersistence, type StoragePersistence } from "../../runtime/storagePersistence";
import { MockDeletionReceipts } from "./MockDeletionReceipts";
import { acquireConversationDeletionOwner, ConversationDeletionController, resetConversationDeletionOwnersForTests } from "./conversationDeletionController";
import { readConversationDeletion } from "./conversationDeletionPending";
import type { ConversationOperationLock } from "./conversationOperationLock";

const id = "00000000-0000-4000-8000-000000000001", epoch = "00000000-0000-4000-8000-000000000002";
const current: AgentConversationSummary = {id, title: "Synthetic original", archived: true, durableCount: 2, createdAt: "now", updatedAt: "now"};
function fixture() {
  const values = new Map<string, string>();
  const storage: Storage = {get length() {return values.size;}, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => {values.set(key, value);}, removeItem: key => {values.delete(key);}, clear: () => values.clear()};
  const occupied = new Set<string>();
  const lock: ConversationOperationLock = {async run(name, operation) {
    if (occupied.has(name)) throw new Error("DELETION_RECOVERY_LOCK_BUSY");
    occupied.add(name); try {return await operation();} finally {occupied.delete(name);}
  }};
  const receipts = new MockDeletionReceipts(() => epoch), mutation = vi.fn(), cleanup = vi.fn();
  const status = vi.fn(async (request: AgentDeletionRequest) => receipts.status(request));
  const remove = vi.fn<AgentDeletionGateway["delete"]>(async (request, options) => {
    receipts.validateWrite(request, options); receipts.apply(request, mutation);
    return {schemaVersion: 1, requestId: request.requestId, conversationId: request.conversationId};
  });
  const gateway: AgentDeletionGateway = {delete: remove, status};
  const ports = {deletions: gateway, cleanup, lock};
  const open = () => new ConversationDeletionController(id, ports, storage);
  return {values, storage, lock, receipts, mutation, cleanup, status, remove, gateway, ports, open};
}
function persistent(storage: Storage) {
  const port: StoragePersistence = {backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush: vi.fn(async () => undefined), assertCurrent: vi.fn(async () => undefined), retry: vi.fn(async () => undefined),
    reset: vi.fn(async () => undefined), subscribe: () => () => undefined, close: vi.fn()};
  registerStoragePersistence(storage, port); return port;
}
it("reliably saves the entire original confirmation and epoch before write, then cleans only after exact ACK", async () => {
  const f = fixture(), owner = f.open();
  f.status.mockImplementation(async request => {
    expect(readConversationDeletion(f.storage, id)?.intent).toEqual(request);
    expect(f.remove).not.toHaveBeenCalled(); return f.receipts.status(request);
  });
  const remove = f.remove.getMockImplementation()!;
  f.remove.mockImplementation(async (request, options) => {
    expect(readConversationDeletion(f.storage, id)).toMatchObject({intent: request, epoch, stage: "unknown"});
    expect(options).toEqual({expectedEpoch: epoch}); expect(f.cleanup).not.toHaveBeenCalled();
    return remove(request, options);
  });
  expect(await owner.submit(current, null)).toBe(true);
  expect(owner.getSnapshot().phase).toBe("ready"); expect(f.cleanup).toHaveBeenCalledExactlyOnceWith(id);
  expect(f.mutation).toHaveBeenCalledOnce(); expect(f.storage.length).toBe(0);
});
it("reopens a lost ACK without mount IPC, inspects the original proof and never repeats deletion", async () => {
  const f = fixture(), owner = f.open(), remove = f.remove.getMockImplementation()!;
  f.remove.mockImplementationOnce(async (...args) => {await remove(...args); throw "lost_ack";});
  expect(await owner.submit(current, null)).toBe(false); const original = owner.getSnapshot().record!;
  owner.dispose(); const reopened = f.open();
  expect(f.status).toHaveBeenCalledOnce(); expect(await reopened.retryOriginal()).toBe(false);
  await reopened.check(); expect(reopened.getSnapshot().phase).toBe("committed");
  expect(await reopened.finish()).toBe(true); expect(f.remove).toHaveBeenCalledOnce(); expect(f.cleanup).toHaveBeenCalledOnce();
  expect(f.status).toHaveBeenLastCalledWith(original.intent);
});
it("does not treat absent SQL proof as absence of external cleanup or allow a new identity after Partial", async () => {
  const f = fixture(), owner = f.open();
  f.remove.mockRejectedValueOnce({code: "storage", reason: {code: "DATA_OPERATION_PARTIAL"}});
  await owner.submit(current, null, {force: true}); const original = owner.getSnapshot().record!;
  expect(await owner.cancelKnown()).toBe(false); expect(await owner.confirmForce()).toBe(false);
  await owner.check(); expect(owner.getSnapshot().phase).toBe("retryable");
  f.remove.mockRejectedValueOnce("notArchived"); await owner.retryOriginal();
  expect(owner.getSnapshot().record).toMatchObject({stage: "unknown", intent: original.intent, reason: null});
  expect(await owner.cancelKnown()).toBe(false); expect(f.cleanup).not.toHaveBeenCalled();
  expect(f.remove.mock.calls[0]![0]).toEqual(f.remove.mock.calls[1]![0]);
});
it("requires an authoritative ordinary retention and a separate force action, retaining all original fields", async () => {
  const f = fixture(), owner = f.open(); f.remove.mockRejectedValueOnce("executionWorkspace");
  await owner.submit(current, {kind: "note", resourceId: "original"}); const normal = owner.getSnapshot().record!;
  expect(normal).toMatchObject({stage: "retained", reason: "executionWorkspace", intent: {force: false}});
  owner.dispose(); const reopened = f.open();
  expect(await reopened.retryOriginal()).toBe(false); expect(f.remove).toHaveBeenCalledOnce();
  expect(await reopened.confirmForce()).toBe(true);
  const forced = f.remove.mock.calls[1]![0]; expect(forced).toEqual({...normal.intent, force: true});
  expect(f.remove.mock.calls[1]![1]).toEqual({expectedEpoch: epoch}); expect(f.cleanup).toHaveBeenCalledOnce();
});
it("blocks IPC on initial storage failure and retries local persistence without creating another request or deleting", async () => {
  const f = fixture(), owner = f.open(), save = vi.spyOn(f.storage, "setItem").mockImplementation(() => {throw Error("quota");});
  await owner.submit(current, null); const original = owner.getSnapshot().record!;
  expect(f.status).not.toHaveBeenCalled(); expect(f.remove).not.toHaveBeenCalled();
  save.mockRestore(); expect(await owner.retryLocal()).toBe(false);
  expect(owner.getSnapshot().phase).toBe("retryable"); expect(f.remove).not.toHaveBeenCalled();
  expect(await owner.retryOriginal()).toBe(true); expect(f.remove.mock.calls[0]![0]).toEqual(original.intent);
});
it("retains the original cleanup owner after view release and retries local failure without another IPC", async () => {
  const f = fixture(); f.cleanup.mockRejectedValueOnce(Error("local cleanup failure"));
  const lease = acquireConversationDeletionOwner(id, f.ports, f.storage);
  await lease.controller.submit(current, null); lease.release();
  const otherCleanup = vi.fn();
  const reopened = acquireConversationDeletionOwner(id, {...f.ports, cleanup: otherCleanup}, f.storage);
  expect(reopened.controller).toBe(lease.controller);
  expect(await reopened.controller.retryLocal()).toBe(true);
  expect(f.cleanup).toHaveBeenCalledTimes(2); expect(otherCleanup).not.toHaveBeenCalled();
  expect(f.status).toHaveBeenCalledOnce(); expect(f.remove).toHaveBeenCalledOnce();
  reopened.release(); resetConversationDeletionOwnersForTests(f.storage, f.gateway);
});
it("restores the committed record when durable removal fails and holds the lock through local flush", async () => {
  const f = fixture(), persistence = persistent(f.storage), owner = f.open();
  vi.mocked(persistence.flush).mockImplementation(async () => {if (f.storage.length === 0) throw Error("remove ACK lost");});
  expect(await owner.submit(current, null)).toBe(false);
  expect(readConversationDeletion(f.storage, id)?.stage).toBe("committed");
  vi.mocked(persistence.flush).mockResolvedValue(undefined);
  expect(await owner.retryLocal()).toBe(true); expect(f.remove).toHaveBeenCalledOnce(); expect(f.storage.length).toBe(0);
});
it("requires original data and proof after reopen, even when local retry is requested", async () => {
  const f = fixture(), owner = f.open(); f.cleanup.mockRejectedValueOnce(Error("keep record"));
  await owner.submit(current, null); owner.dispose(); const reopened = f.open();
  f.status.mockImplementation(async request => ({schemaVersion: 1, operationId: request.requestId, conversationId: id,
    status: {type: "noSqlReceipt", epoch: crypto.randomUUID(), sqlFence: "0", closed: false}}));
  await reopened.check(); expect(reopened.getSnapshot().cause).toMatchObject({message: "DELETION_RECOVERY_EPOCH_CHANGED"});
  expect(await reopened.retryLocal()).toBe(false); expect(f.cleanup).toHaveBeenCalledOnce();
  f.status.mockImplementation(async request => ({schemaVersion: 1, operationId: request.requestId, conversationId: id,
    status: {type: "noSqlReceipt", epoch, sqlFence: "0", closed: false}}));
  await reopened.check(); expect(reopened.getSnapshot().cause).toMatchObject({message: "DELETION_RECOVERY_PROOF_MISSING"});
  expect(await reopened.retryOriginal()).toBe(false); expect(readConversationDeletion(f.storage, id)?.stage).toBe("committed");
});
it("excludes a second window throughout IPC and lets it discover the original uncertain request afterwards", async () => {
  const f = fixture(), first = f.open(), second = f.open();
  let finish!: () => void, entered!: () => void;
  const active = new Promise<void>(resolve => {entered = resolve;}), gate = new Promise<void>(resolve => {finish = resolve;});
  const remove = f.remove.getMockImplementation()!;
  f.remove.mockImplementationOnce(async (...args) => {entered(); await gate; await remove(...args); throw "lost_ack";});
  const pending = first.submit(current, null); await active;
  const saved = new Map(f.values); expect(await second.submit(current, null)).toBe(false);
  expect(second.getSnapshot().localIssue).toMatchObject({message: "DELETION_RECOVERY_LOCK_BUSY"});
  expect(f.values).toEqual(saved); expect(f.status).toHaveBeenCalledOnce();
  finish(); await pending;
  expect(await second.submit(current, null)).toBe(false); expect(second.getSnapshot().record?.intent).toEqual(first.getSnapshot().record?.intent);
  await second.check(); expect(await second.finish()).toBe(true); expect(f.remove).toHaveBeenCalledOnce();
});
it("blocks unsupported locks and stale hydrated storage before allocation or IPC", async () => {
  const f = fixture(); const owner = new ConversationDeletionController(id, {...f.ports,
    lock: {run: async () => {throw Error("DELETION_RECOVERY_LOCK_UNAVAILABLE");}}}, f.storage);
  await owner.submit(current, null); expect(f.storage.length).toBe(0); expect(f.status).not.toHaveBeenCalled();
  const persistence = persistent(f.storage); vi.mocked(persistence.assertCurrent!).mockRejectedValue(Error("stale preview"));
  await f.open().submit(current, null); expect(f.storage.length).toBe(0); expect(f.status).not.toHaveBeenCalled();
});

it("keeps an in-flight deletion and its original cleanup alive after the last view releases", async () => {
  const f = fixture(), lease = acquireConversationDeletionOwner(id, f.ports, f.storage);
  let finish!: () => void, entered!: () => void;
  const active = new Promise<void>(resolve => {entered = resolve;}), gate = new Promise<void>(resolve => {finish = resolve;});
  const remove = f.remove.getMockImplementation()!;
  f.remove.mockImplementationOnce(async (...args) => {entered(); await gate; return remove(...args);});
  const pending = lease.controller.submit(current, null); await active; lease.release();
  const cleanup = vi.fn(), next = acquireConversationDeletionOwner(id, {...f.ports, cleanup}, f.storage);
  expect(next.controller).toBe(lease.controller); finish(); expect(await pending).toBe(true);
  expect(f.cleanup).toHaveBeenCalledExactlyOnceWith(id); expect(cleanup).not.toHaveBeenCalled();
  next.release(); resetConversationDeletionOwnersForTests(f.storage, f.gateway);
});

it("retains a valid native ACK if saving its committed marker fails and retries only local work", async () => {
  const f = fixture(), owner = f.open(), set = f.storage.setItem.bind(f.storage);
  const save = vi.spyOn(f.storage, "setItem").mockImplementation((key, value) => {
    if (JSON.parse(value).stage === "committed") throw Error("quota after native ACK");
    set(key, value);
  });
  expect(await owner.submit(current, null)).toBe(false);
  expect(owner.getSnapshot().record?.stage).toBe("committed");
  expect(readConversationDeletion(f.storage, id)?.stage).toBe("unknown"); expect(f.cleanup).not.toHaveBeenCalled();
  save.mockRestore(); expect(await owner.retryLocal()).toBe(true);
  expect(f.status).toHaveBeenCalledOnce(); expect(f.remove).toHaveBeenCalledOnce(); expect(f.cleanup).toHaveBeenCalledOnce();
});

it("never overwrites another request installed after this owner observed the original slot", async () => {
  const f = fixture(), owner = f.open(); f.remove.mockRejectedValueOnce("lost_before_commit");
  await owner.submit(current, null); const key = f.storage.key(0)!, original = owner.getSnapshot().record!;
  const replacement = {...original, intent: {...original.intent, requestId: crypto.randomUUID()}};
  f.storage.setItem(key, JSON.stringify(replacement)); const saved = f.storage.getItem(key);
  await owner.check(); await owner.retryLocal(); await owner.cancelKnown();
  expect(owner.getSnapshot().localIssue).toMatchObject({message: "DELETION_RECOVERY_CONFLICT"});
  expect(f.storage.getItem(key)).toBe(saved); expect(f.remove).toHaveBeenCalledOnce(); expect(f.status).toHaveBeenCalledOnce();
});

it("lets an authoritative first Busy rejection be canceled without cleaning the conversation", async () => {
  const f = fixture(), owner = f.open(); f.remove.mockRejectedValueOnce("busy");
  await owner.submit(current, null); expect(owner.getSnapshot().record).toMatchObject({stage: "rejected", reason: "busy"});
  expect(await owner.cancelKnown()).toBe(true); expect(f.cleanup).not.toHaveBeenCalled(); expect(f.storage.length).toBe(0);
});

it("repairs a failed retention save before permitting the separate force confirmation", async () => {
  const f = fixture(), owner = f.open(), set = f.storage.setItem.bind(f.storage);
  const save = vi.spyOn(f.storage, "setItem").mockImplementation((key, value) => {
    if (JSON.parse(value).stage === "retained") throw Error("retention save failure");
    set(key, value);
  });
  f.remove.mockRejectedValueOnce("executionWorkspace"); await owner.submit(current, null);
  expect(owner.getSnapshot().localIssue).not.toBeNull(); const original = owner.getSnapshot().record!;
  expect(await owner.confirmForce()).toBe(false); expect(owner.getSnapshot().record).toEqual(original);
  expect(f.remove).toHaveBeenCalledOnce(); save.mockRestore();
  await owner.retryLocal(); expect(owner.getSnapshot().phase).toBe("retained");
  expect(await owner.confirmForce()).toBe(true); expect(f.remove.mock.calls[1]![0]).toEqual({...original.intent, force: true});
});

it("releases an owner that never allocated an intent rather than retaining a dead view's cleanup", async () => {
  const f = fixture(), lease = acquireConversationDeletionOwner(id, {...f.ports,
    lock: {run: async () => {throw Error("DELETION_RECOVERY_LOCK_BUSY");}}}, f.storage);
  await lease.controller.submit(current, null); expect(lease.controller.getSnapshot().record).toBeNull(); lease.release();
  const cleanup = vi.fn(), next = acquireConversationDeletionOwner(id, {...f.ports, cleanup}, f.storage);
  expect(next.controller).not.toBe(lease.controller); expect(await next.controller.submit(current, null)).toBe(true);
  expect(cleanup).toHaveBeenCalledExactlyOnceWith(id); expect(f.cleanup).not.toHaveBeenCalled();
  next.release(); resetConversationDeletionOwnersForTests(f.storage, f.gateway);
});
