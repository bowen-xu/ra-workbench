import { createTestConversationForkLock } from "./conversationForkTestLock";
import { expect, it, vi } from "vitest";
import type { AgentForkRequest, AgentForkStatusResponse, AgentConversationSummary, AgentForkWriteOptions } from "@research-assistant/core/agent";
import { ConversationForkController, acquireConversationForkOwner, resetConversationForkOwnersForTests } from "./conversationForkController";
import { registerStoragePersistence } from "../../runtime/storagePersistence";
import { readConversationFork } from "./conversationForkPending";

const epoch = "00000000-0000-4000-8000-000000000009";
const scope = { parentId: "00000000-0000-4000-8000-000000000001", throughTurnId: "00000000-0000-4000-8000-000000000002", binding: { kind: "note" as const, resourceId: "synthetic" } };
function fixture() {
  const values = new Map<string, string>();
  const storage: Storage = { get length() { return values.size; }, key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); }, clear: () => values.clear() };
  const committed = new Set<string>(); let currentEpoch = epoch, closed = false;
  const status = vi.fn(async (request: AgentForkRequest): Promise<AgentForkStatusResponse> => ({ schemaVersion: 1, operationId: request.childId, conversationId: request.childId,
    status: committed.has(request.childId) ? { type: "committed", proof: { schemaVersion: 1, operationId: request.childId, conversationId: request.childId, family: "agent.conversation.fork",
      epoch: currentEpoch, sqlFence: "7", replayed: true, result: null } } : { type: "noSqlReceipt", epoch: currentEpoch, sqlFence: "0", closed } }));
  const summary = (id: string): AgentConversationSummary => ({ id, title: "Later name", archived: true, durableCount: 4, createdAt: "2026-10-27T00:00:00.000Z", updatedAt: "2026-10-27T00:00:00.000Z" });
  const fork = vi.fn(async (request: AgentForkRequest, _options?: AgentForkWriteOptions) => { committed.add(request.childId);
    return { schemaVersion: 1 as const, conversation: summary(request.childId), parentId: request.parentId, throughTurnId: request.throughTurnId, parentEndSeq: 2 }; });
  const read = vi.fn(async ({ id }: { id: string }) => summary(id));
  const ports = { fork: { fork, status }, metadata: { read }, lock: createTestConversationForkLock() };
  return { values, storage, ports, fork, status, read, summary, controller: new ConversationForkController(scope, ports, storage),
    commit: () => { const intent = readConversationFork(storage,scope)?.intent; if (intent) committed.add(intent.childId); }, resetEpoch: () => { currentEpoch = crypto.randomUUID(); }, close: () => { closed = true; } };
}
it("saves the complete intent and freezes epoch before mutation; proof and current archived child stay separate", async () => {
  const f = fixture();
  f.fork.mockImplementationOnce(async request => {
    expect(readConversationFork(f.storage, scope)).toEqual({ schemaVersion: 1, intent: request, epoch, stage: "unknown" });
    f.commit(); return { schemaVersion: 1, conversation: f.summary(request.childId), parentId: scope.parentId, throughTurnId: scope.throughTurnId, parentEndSeq: 2 };
  });
  expect((await f.controller.submit())?.archived).toBe(true);
  expect(f.status).toHaveBeenCalledTimes(2); expect(f.fork).toHaveBeenCalledOnce();
  const snapshot = f.controller.getSnapshot();
  expect(snapshot.proof?.result).toBeNull(); expect(snapshot.current?.title).toBe("Later name");
  expect(snapshot.proof?.conversationId).toBe(snapshot.record?.intent.childId);
  const opened = vi.fn(); expect(await f.controller.finish(opened)).toBe(true);
  expect(opened).toHaveBeenCalledWith(snapshot.current); expect(readConversationFork(f.storage, scope)).toBeNull();
});
it("recovers lost ACK after a cold view by readonly inspection and reads current metadata without republishing", async () => {
  const f = fixture(); f.fork.mockImplementationOnce(async () => { f.commit(); throw "lost_ack"; });
  await f.controller.submit(); const original = readConversationFork(f.storage, scope)!;
  f.controller.dispose(); const reopened = new ConversationForkController(scope, f.ports, f.storage);
  expect(reopened.getSnapshot().phase).toBe("unknown"); expect(f.status).toHaveBeenCalledOnce();
  await reopened.check(); expect(reopened.getSnapshot().proof?.operationId).toBe(original.intent.childId);
  expect(f.read).not.toHaveBeenCalled(); await reopened.readCurrent();
  expect(f.fork).toHaveBeenCalledOnce(); expect(reopened.getSnapshot().current?.archived).toBe(true);
});
it("allows only explicit same-intent retry after no receipt; closed and unconfirmed statuses cannot authorize a write", async () => {
  const f = fixture(); f.fork.mockRejectedValueOnce("before_commit_unknown"); await f.controller.submit();
  const original = structuredClone(f.fork.mock.calls[0]![0]);
  expect(await f.controller.retryOriginal()).toBeNull(); await f.controller.check();
  expect(f.fork).toHaveBeenCalledOnce(); await f.controller.retryOriginal();
  expect(f.fork.mock.calls[1]![0]).toEqual(original);
  const g = fixture(); g.close(); await g.controller.submit(); expect(g.fork).not.toHaveBeenCalled();
  const h = fixture(); h.status.mockImplementation(async request => ({ schemaVersion: 1, operationId: request.childId, conversationId: request.childId,
    status: { type: "unconfirmed", epoch, sqlFence: "0", closed: false, blocked: true, reason: "commit_unknown" } }));
  await h.controller.submit(); await h.controller.retryOriginal(); expect(h.fork).not.toHaveBeenCalled();
});
it("refuses a changed data epoch and never downgrades a previously committed record", async () => {
  const f = fixture(); f.fork.mockRejectedValueOnce("lost_ack"); await f.controller.submit(); f.resetEpoch();
  await f.controller.check(); await f.controller.retryOriginal(); expect(f.fork).toHaveBeenCalledOnce();
  expect(f.controller.getSnapshot().cause).toEqual(new Error("FORK_RECOVERY_EPOCH_CHANGED"));
  const g = fixture(); await g.controller.submit();
  g.status.mockImplementation(async request => ({ schemaVersion: 1, operationId: request.childId, conversationId: request.childId,
    status: { type: "noSqlReceipt", epoch, sqlFence: "0", closed: false } }));
  const reopened = new ConversationForkController(scope, g.ports, g.storage); await reopened.check(); await reopened.retryOriginal();
  expect(reopened.getSnapshot().cause).toEqual(new Error("FORK_RECOVERY_PROOF_MISSING")); expect(g.fork).toHaveBeenCalledOnce();
});
it("preserves malformed records and stops before IPC when saving the original request fails", async () => {
  const f = fixture(); vi.spyOn(f.storage, "setItem").mockImplementation(() => { throw new Error("quota"); });
  await f.controller.submit(); expect(f.status).not.toHaveBeenCalled(); expect(f.fork).not.toHaveBeenCalled();
  const g = fixture(); g.fork.mockRejectedValueOnce("lost_ack"); await g.controller.submit();
  const locator = [...g.values.keys()][0]!; g.values.set(locator, "private invalid JSON");
  const reopened = new ConversationForkController(scope, g.ports, g.storage); await reopened.submit();
  expect(reopened.getSnapshot().localIssue).toEqual(new Error("FORK_RECOVERY_INVALID"));
  expect(g.values.get(locator)).toBe("private invalid JSON"); expect(g.fork).toHaveBeenCalledOnce();
});
it("keeps the proof when current metadata is missing and cleans up only the exact record", async () => {
  const f = fixture(); f.read.mockRejectedValue("notFound"); await f.controller.submit();
  expect(f.controller.getSnapshot().proof).not.toBeNull(); expect(f.controller.getSnapshot().phase).toBe("readError");
  const remove = vi.spyOn(f.storage, "removeItem").mockImplementationOnce(() => { throw new Error("quota"); });
  expect(await f.controller.finish()).toBe(false); expect(readConversationFork(f.storage, scope)).not.toBeNull();
  remove.mockRestore(); expect(await f.controller.finish()).toBe(true); expect(readConversationFork(f.storage, scope)).toBeNull();
  expect(f.controller.getSnapshot()).toMatchObject({phase:"idle",record:null,proof:null});
});
it("retains one in-flight owner across view release and forbids duplicate submissions", async () => {
  const f = fixture(); let release!: () => void;
  f.fork.mockImplementationOnce(async request => { await new Promise<void>(resolve => { release = resolve; }); f.commit();
    return { schemaVersion: 1, conversation: f.summary(request.childId), parentId: scope.parentId, throughTurnId: scope.throughTurnId, parentEndSeq: 2 }; });
  const first = acquireConversationForkOwner(scope, f.ports, f.storage), work = first.controller.submit();
  await vi.waitFor(() => expect(f.fork).toHaveBeenCalledOnce()); first.release();
  const next = acquireConversationForkOwner(scope, f.ports, f.storage); expect(next.controller).toBe(first.controller);
  await next.controller.submit(); expect(f.fork).toHaveBeenCalledOnce(); release(); await work;
  next.release(); resetConversationForkOwnersForTests(f.storage);
});
it("waits for the original IndexedDB receipt before any status/write IPC and retries local failure explicitly", async () => {
  const f = fixture(); let release!: () => void;
  const flush = vi.fn(async () => {}), retry = vi.fn(async () => {});
  flush.mockImplementationOnce(() => new Promise<void>(resolve => { release = resolve; }));
  registerStoragePersistence(f.storage, { backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush, retry, assertCurrent: async () => {}, reset: async () => {}, close() {}, subscribe: () => () => {} });
  const work = f.controller.submit();
  expect(f.status).not.toHaveBeenCalled(); expect(f.fork).not.toHaveBeenCalled();
  await vi.waitFor(() => expect(flush).toHaveBeenCalledOnce());
  const original = readConversationFork(f.storage, scope)!.intent;
  release(); await work;
  expect(f.fork).toHaveBeenCalledExactlyOnceWith(original, {expectedEpoch: epoch});
  expect(flush).toHaveBeenCalledTimes(4); // request, epoch, pre-send Unknown, proof
  flush.mockRejectedValueOnce(new Error("IDB_CLEAR_UNCONFIRMED"));
  expect(await f.controller.finish()).toBe(false);
  expect(readConversationFork(f.storage, scope)?.intent).toEqual(original);
  expect(retry).not.toHaveBeenCalled(); await f.controller.retryLocal();
  expect(retry).toHaveBeenCalledOnce(); expect(f.fork).toHaveBeenCalledOnce();
  await f.controller.readCurrent(); expect(await f.controller.finish()).toBe(true);
  expect(readConversationFork(f.storage, scope)).toBeNull();
});
it("never sends after a failed pre-send durable flush and does not overwrite a foreign recovery record", async () => {
  const f = fixture(), flush = vi.fn(async () => {}), retry = vi.fn(async () => {});
  flush.mockResolvedValueOnce().mockResolvedValueOnce().mockRejectedValueOnce(new Error("IDB_BEFORE_IPC_UNCONFIRMED"));
  registerStoragePersistence(f.storage, { backend: "IndexedDB", pending: false, error: undefined, resetting: false,
    flush, retry, assertCurrent: async () => {}, reset: async () => {}, close() {}, subscribe: () => () => {} });
  await f.controller.submit(); const original = readConversationFork(f.storage, scope)!;
  expect(f.fork).not.toHaveBeenCalled(); await f.controller.retryOriginal(); expect(f.fork).not.toHaveBeenCalled();
  await f.controller.retryLocal(); await f.controller.retryOriginal(); expect(f.fork).toHaveBeenCalledExactlyOnceWith(original.intent, {expectedEpoch: epoch});
  const locator = [...f.values.keys()][0]!, foreign = { ...readConversationFork(f.storage, scope)!, intent: { ...original.intent, childId: crypto.randomUUID() } };
  f.values.set(locator, JSON.stringify(foreign)); const adopt = vi.fn();
  expect(await f.controller.finish(adopt)).toBe(false); expect(adopt).not.toHaveBeenCalled();
  expect(readConversationFork(f.storage, scope)).toEqual(foreign);
});
it("carries the original epoch through a data switch after preflight and retains that original after the backend rejects the write", async () => {
  const f = fixture();
  f.fork.mockImplementationOnce(async (request, options) => {
    expect(options).toEqual({expectedEpoch: epoch}); f.resetEpoch();
    throw {code:"storage",reason:{code:"DATA_SQL_OPERATION_CONFLICT",reason:{operationId:request.childId,phase:"fork-epoch",safeCause:"data_epoch_changed"}}};
  });
  await f.controller.submit(); const original = readConversationFork(f.storage, scope)!;
  expect(original.epoch).toBe(epoch); expect(original.stage).toBe("unknown");
  expect(f.controller.getSnapshot().phase).toBe("unknown"); expect(f.read).not.toHaveBeenCalled();
  await f.controller.check(); await f.controller.retryOriginal();
  expect(f.fork).toHaveBeenCalledOnce(); expect(readConversationFork(f.storage, scope)).toEqual(original);
  expect(f.controller.getSnapshot().cause).toEqual(new Error("FORK_RECOVERY_EPOCH_CHANGED"));
});
it("lets independent window actors share one original child and recover a lost ACK without a second allocation", async () => {
  const f = fixture(), second = new ConversationForkController(scope, f.ports, f.storage); let release!: () => void;
  f.fork.mockImplementationOnce(async () => { await new Promise<void>(resolve => { release = resolve; }); f.commit(); throw "lost_ack"; });
  const first = f.controller.submit(); await vi.waitFor(() => expect(f.fork).toHaveBeenCalledOnce());
  const original = readConversationFork(f.storage, scope)!;
  await second.submit(); expect(second.getSnapshot().localIssue).toEqual(new Error("FORK_RECOVERY_LOCK_BUSY"));
  expect(second.getSnapshot().record).toBeNull(); expect(readConversationFork(f.storage, scope)).toEqual(original);
  release(); await first; await second.submit(); expect(second.getSnapshot().record?.intent).toEqual(original.intent);
  await second.check(); await second.readCurrent(); expect(second.getSnapshot().phase).toBe("ready");
  expect(f.fork).toHaveBeenCalledOnce();
});
it("holds the same lock through durable removal; a later explicit new branch receives a new identity", async () => {
  const f = fixture(); await f.controller.submit(); const original = readConversationFork(f.storage, scope)!;
  let release!: () => void;
  const flush = vi.fn(async () => {}); flush.mockImplementationOnce(() => new Promise<void>(resolve => { release = resolve; }));
  registerStoragePersistence(f.storage, {backend:"IndexedDB",pending:false,error:undefined,resetting:false,
    flush,assertCurrent:async()=>{},retry:async()=>{},reset:async()=>{},close(){},subscribe:()=>()=>{}});
  const finish = f.controller.finish(); await vi.waitFor(() => expect(flush).toHaveBeenCalledOnce());
  const second = new ConversationForkController(scope, f.ports, f.storage);
  await second.submit(); expect(f.fork).toHaveBeenCalledOnce(); expect(second.getSnapshot().localIssue).toEqual(new Error("FORK_RECOVERY_LOCK_BUSY"));
  release(); expect(await finish).toBe(true); await second.submit(); expect(f.fork).toHaveBeenCalledTimes(2);
  expect(f.fork.mock.calls[1]![0].childId).not.toBe(original.intent.childId);
});
it("does not let a stale same-intent retry downgrade another window's committed record", async () => {
  const f = fixture(); f.fork.mockRejectedValueOnce("before_commit"); await f.controller.submit();
  const second = new ConversationForkController(scope, f.ports, f.storage); await second.check();
  await f.controller.check(); await f.controller.retryOriginal();
  const committed = readConversationFork(f.storage, scope)!; expect(committed.stage).toBe("committed");
  await second.retryOriginal(); expect(f.fork).toHaveBeenCalledTimes(2);
  expect(readConversationFork(f.storage, scope)).toEqual(committed);
  expect(second.getSnapshot().localIssue).toEqual(new Error("FORK_RECOVERY_CONFLICT"));
});
it("blocks before allocation or IPC when cross-window locks or a current storage view are unavailable", async () => {
  const f = fixture(), unsupported = new ConversationForkController(scope,{...f.ports,lock:{async run(){throw Error("FORK_RECOVERY_LOCK_UNAVAILABLE");}}},f.storage);
  await unsupported.submit(); expect(f.values.size).toBe(0); expect(f.fork).not.toHaveBeenCalled(); expect(f.status).not.toHaveBeenCalled();
  expect(unsupported.getSnapshot().localIssue).toEqual(new Error("FORK_RECOVERY_LOCK_UNAVAILABLE"));
  const stale = fixture(); registerStoragePersistence(stale.storage,{backend:"IndexedDB",pending:false,error:undefined,resetting:false,
    flush:async()=>{},assertCurrent:async()=>{throw Error("FORK_RECOVERY_STORAGE_STALE");},retry:async()=>{},reset:async()=>{},close(){},subscribe:()=>()=>{}});
  await stale.controller.submit(); expect(stale.values.size).toBe(0); expect(stale.fork).not.toHaveBeenCalled();
  expect(stale.controller.getSnapshot().localIssue).toEqual(new Error("FORK_RECOVERY_STORAGE_STALE"));
});
