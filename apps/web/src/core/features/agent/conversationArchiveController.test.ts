/** @vitest-environment happy-dom */
import { afterEach, expect, it, vi } from "vitest";
import { AGENT_ARCHIVE_FAMILY, type AgentArchiveProof, type AgentArchiveRequest, type AgentArchiveStatus,
  type AgentConversationSummary } from "@research-assistant/core/agent";
import { registerStoragePersistence, type StoragePersistence } from "../../runtime/storagePersistence";
import { ConversationArchiveController, acquireConversationArchiveOwner, archiveUnresolved, resetConversationArchiveOwnersForTests } from "./conversationArchiveController";
import { clearConversationArchive, discoverConversationArchives, readConversationArchive, saveConversationArchive } from "./conversationArchivePending";
import { archiveRecoveryMessage, conversationArchiveMessage } from "./conversationArchiveMessage";

const id = "11111111-1111-4111-8111-111111111111";
const context = { binding: { kind: "paper" as const, resourceId: "CaseSensitivePaper" }, title: "Synthetic", internal: false };
const summary: AgentConversationSummary = { id, title: context.title, archived: false, durableCount: 14,
  createdAt: "2026-10-26T00:00:00.000Z", updatedAt: "2026-10-26T00:00:00.000Z" };
function proof(request: AgentArchiveRequest): AgentArchiveProof {
  return { schemaVersion: 1, operationId: request.operationId, conversationId: request.id,
    family: AGENT_ARCHIVE_FAMILY, epoch: "22222222-2222-4222-8222-222222222222", sqlFence: "7", replayed: false, result: null };
}
class MemoryStorage implements Storage {
  private readonly items = new Map<string, string>();
  get length() { return this.items.size; }
  key(index: number) { return [...this.items.keys()][index] ?? null; }
  getItem(key: string) { return this.items.get(key) ?? null; }
  setItem(key: string, value: string) { this.items.set(key, value); }
  removeItem(key: string) { this.items.delete(key); }
  clear() { this.items.clear(); }
}
const storages: Storage[] = [];

it("reports a canceled preview admission without claiming the original operation failed", () => {
  const message = conversationArchiveMessage("AGENT_PREVIEW_SCENARIO_CANCELED");
  expect(message).toContain("本次归档重试未被接纳");
  expect(message).toContain("原操作仍待核对");
  expect(message).toContain("AGENT_PREVIEW_SCENARIO_CANCELED");
  expect(conversationArchiveMessage("unavailable")).toContain("unavailable");
});
function fixture() {
  const storage = new MemoryStorage(); storages.push(storage);
  const setArchived = vi.fn(async (intent: AgentArchiveRequest) => proof(intent));
  const archiveStatus = vi.fn(async (intent: AgentArchiveRequest) => ({ schemaVersion: 1 as const, operationId: intent.operationId,
    conversationId: intent.id, status: { type: "committed" as const, proof: proof(intent) } as AgentArchiveStatus }));
  const read = vi.fn(async () => ({ ...summary, archived: true }));
  const ports = { archives: { setArchived, archiveStatus }, metadata: { read } };
  return { storage, setArchived, archiveStatus, read, ports, controller: new ConversationArchiveController(id, ports, storage) };
}
function deferred<T>() { let resolve!: (value: T) => void; const promise = new Promise<T>(done => { resolve = done; }); return { promise, resolve }; }
afterEach(() => { for (const storage of storages.splice(0)) resetConversationArchiveOwnersForTests(storage); vi.restoreAllMocks(); });

it("freezes archive W from the row and adopts current metadata independently of the historical proof", async () => {
  const f = fixture(); f.read.mockResolvedValue({ ...summary, title: "Later restored title", durableCount: 29, archived: false });
  expect(await f.controller.submit(summary, context)).toMatchObject({ archived: false, title: "Later restored title", durableCount: 29 });
  const request = f.setArchived.mock.calls[0]![0];
  expect(request).toEqual({ operationId: expect.any(String), id, archived: true, expectedWatermark: 14 });
  expect(f.read).toHaveBeenCalledExactlyOnceWith({ id, binding: context.binding });
  expect(readConversationArchive(f.storage, id)).toBeNull(); expect(archiveUnresolved(f.controller.getSnapshot())).toBe(false);
  expect(f.archiveStatus).not.toHaveBeenCalled();
});

it("restores with null W and no history/metadata read before dispatch", async () => {
  const f = fixture(); f.read.mockResolvedValue(summary);
  await f.controller.submit({ ...summary, archived: true, durableCount: Number.MAX_SAFE_INTEGER + 1 }, context);
  expect(f.setArchived.mock.calls[0]![0]).toMatchObject({ archived: false, expectedWatermark: null });
  expect(f.setArchived.mock.invocationCallOrder[0]).toBeLessThan(f.read.mock.invocationCallOrder[0]!);
});

it("reopens Unknown using the exact original intent; explicit status does not mutate or read metadata", async () => {
  const f = fixture(); f.setArchived.mockRejectedValueOnce(new Error("lost reply"));
  await f.controller.submit(summary, context); const intent = f.controller.getSnapshot().intent!;
  const restored = new ConversationArchiveController(id, f.ports, f.storage);
  expect(restored.getSnapshot()).toMatchObject({ phase: "unknown", intent });
  await restored.submit({ ...summary, durableCount: 99 }, context); expect(f.setArchived).toHaveBeenCalledTimes(1);
  expect(f.read).not.toHaveBeenCalled(); expect(f.archiveStatus).not.toHaveBeenCalled();
  await restored.check(); expect(f.archiveStatus).toHaveBeenCalledExactlyOnceWith(intent);
  expect(f.read).not.toHaveBeenCalled(); expect(f.setArchived).toHaveBeenCalledTimes(1);
  await restored.readCurrent(); expect(f.read).toHaveBeenCalledOnce();
  expect(f.setArchived).toHaveBeenCalledTimes(1); expect(readConversationArchive(f.storage, id)).toBeNull();
});

it("retries only explicitly with the frozen original op and W after noSqlReceipt; closed status prohibits replay", async () => {
  const f = fixture(); f.setArchived.mockRejectedValueOnce("unavailable");
  f.archiveStatus.mockImplementation(async intent => ({ schemaVersion: 1, operationId: intent.operationId, conversationId: intent.id,
    status: { type: "noSqlReceipt", epoch: "22222222-2222-4222-8222-222222222222", sqlFence: "8", closed: true } }));
  await f.controller.submit(summary, context); const intent = f.controller.getSnapshot().intent!;
  await f.controller.check(); await f.controller.retryOriginal(); expect(f.setArchived).toHaveBeenCalledTimes(1);
  f.archiveStatus.mockImplementation(async request => ({ schemaVersion: 1, operationId: request.operationId, conversationId: request.id,
    status: { type: "noSqlReceipt", epoch: "22222222-2222-4222-8222-222222222222", sqlFence: "9", closed: false } }));
  await f.controller.check(); await f.controller.retryOriginal();
  expect(f.setArchived.mock.calls.map(([request]) => request)).toEqual([intent, intent]);
});

it("clears an explicitly rejected original retry before a new archive uses the refreshed watermark", async () => {
  const f = fixture();
  f.setArchived.mockRejectedValueOnce(new Error("lost reply")).mockRejectedValueOnce("conflict");
  f.archiveStatus.mockImplementationOnce(async intent => ({schemaVersion: 1, operationId: intent.operationId, conversationId: intent.id,
    status: {type: "noSqlReceipt", epoch: "22222222-2222-4222-8222-222222222222", sqlFence: "8", closed: false}}));
  await f.controller.submit(summary, context);
  const original = f.controller.getSnapshot().intent!;
  await f.controller.check();
  expect(f.controller.getSnapshot()).toMatchObject({phase: "notCommitted", intent: original});
  await f.controller.retryOriginal();
  expect(f.setArchived.mock.calls[1]![0]).toEqual(original);
  expect(f.controller.getSnapshot()).toMatchObject({phase: "rejected", cause: "conflict", intent: null});
  expect(readConversationArchive(f.storage, id)).toBeNull();
  expect(archiveUnresolved(f.controller.getSnapshot())).toBe(false);
  await f.controller.retryOriginal(); expect(f.setArchived).toHaveBeenCalledTimes(2);
  f.read.mockResolvedValue({...summary, durableCount: 18, archived: true});
  const result = await f.controller.submit({...summary, durableCount: 18}, context);
  const next = f.setArchived.mock.calls[2]![0];
  expect(next).toEqual({...original, operationId: expect.any(String), expectedWatermark: 18});
  expect(next.operationId).not.toBe(original.operationId);
  expect(result).toMatchObject({durableCount: 18, archived: true});
  expect(readConversationArchive(f.storage, id)).toBeNull();
});

it("discards an old no-receipt result when a later status check fails", async () => {
  const f = fixture();
  f.setArchived.mockRejectedValueOnce(new Error("lost reply"));
  f.archiveStatus.mockImplementationOnce(async intent => ({schemaVersion: 1, operationId: intent.operationId, conversationId: intent.id,
    status: {type: "noSqlReceipt", epoch: "22222222-2222-4222-8222-222222222222", sqlFence: "8", closed: false}}));
  const failure = Object.assign(new Error("network timeout"), {code: "ARCHIVE_CHECK_TIMEOUT"});
  f.archiveStatus.mockRejectedValueOnce(failure);
  await f.controller.submit(summary, context); const intent = f.controller.getSnapshot().intent!;
  await f.controller.check(); expect(f.controller.getSnapshot().phase).toBe("notCommitted");
  await f.controller.check();
  expect(f.controller.getSnapshot()).toMatchObject({phase: "unknown", intent, status: null, cause: failure});
  expect(archiveRecoveryMessage(f.controller.getSnapshot())).toContain("ARCHIVE_CHECK_TIMEOUT");
  expect(archiveRecoveryMessage(f.controller.getSnapshot())).not.toContain("未找到原操作回执");
  await f.controller.retryOriginal(); expect(f.setArchived).toHaveBeenCalledTimes(1);
  expect(readConversationArchive(f.storage, id)?.intent).toEqual(intent);
});

it("does not reuse a no-receipt result after the original archive retry fails", async () => {
  const f = fixture();
  f.setArchived.mockRejectedValueOnce(new Error("lost reply")).mockRejectedValueOnce(new Error("retry timeout"));
  f.archiveStatus.mockImplementationOnce(async intent => ({schemaVersion: 1, operationId: intent.operationId, conversationId: intent.id,
    status: {type: "noSqlReceipt", epoch: "22222222-2222-4222-8222-222222222222", sqlFence: "8", closed: false}}));
  await f.controller.submit(summary, context); const intent = f.controller.getSnapshot().intent!;
  await f.controller.check(); await f.controller.retryOriginal();
  expect(f.controller.getSnapshot()).toMatchObject({phase: "unknown", intent, status: null});
  expect(archiveRecoveryMessage(f.controller.getSnapshot())).not.toContain("未找到原操作回执");
  await f.controller.retryOriginal(); expect(f.setArchived).toHaveBeenCalledTimes(2);
  expect(readConversationArchive(f.storage, id)?.intent).toEqual(intent);
});

it("identifies the affected conversation and operation in a recoverable notice", async () => {
  const f = fixture();
  f.setArchived.mockRejectedValueOnce(new Error("lost reply"));
  f.archiveStatus.mockImplementationOnce(async intent => ({schemaVersion: 1, operationId: intent.operationId, conversationId: intent.id,
    status: {type: "noSqlReceipt", epoch: "22222222-2222-4222-8222-222222222222", sqlFence: "8", closed: false}}));
  await f.controller.submit({...summary, archived: true}, {...context, title: `标题\n${"长".repeat(45)}`});
  const intent = f.controller.getSnapshot().intent!;
  await f.controller.check();
  const message = archiveRecoveryMessage(f.controller.getSnapshot());
  expect(message).toContain(`“标题 ${"长".repeat(37)}…”的恢复操作`);
  expect(message).toContain("未找到原操作回执，无法据此确认操作是否执行");
  expect(message).toContain(intent.id); expect(message).toContain(intent.operationId);
  expect(message).not.toContain("\n");
});

it("holds known commit through metadata failure and reads again without submitting archive", async () => {
  const f = fixture(); f.read.mockRejectedValueOnce("unavailable");
  await f.controller.submit(summary, context); const intent = f.controller.getSnapshot().intent!;
  expect(f.controller.getSnapshot()).toMatchObject({ phase: "readError", proof: proof(intent) });
  expect(readConversationArchive(f.storage, id)?.stage).toBe("committed");
  await f.controller.submit(summary, context); await f.controller.check(); await f.controller.retryOriginal(); expect(f.setArchived).toHaveBeenCalledTimes(1);
  await f.controller.readCurrent(); expect(f.read).toHaveBeenCalledTimes(2); expect(f.archiveStatus).not.toHaveBeenCalled();
});

it("does not downgrade a recorded commit or retry it when the current environment has no receipt", async () => {
  const f = fixture(); f.read.mockRejectedValueOnce("unavailable");
  await f.controller.submit(summary, context);
  const saved = readConversationArchive(f.storage, id)!;
  expect(saved.stage).toBe("committed");
  const restored = new ConversationArchiveController(id, f.ports, f.storage);
  expect(restored.getSnapshot()).toMatchObject({phase: "unknown", intent: saved.intent, recordedCommitted: true});
  f.archiveStatus.mockImplementationOnce(async intent => ({schemaVersion: 1, operationId: intent.operationId, conversationId: intent.id,
    status: {type: "noSqlReceipt", epoch: "33333333-3333-4333-8333-333333333333", sqlFence: "0", closed: false}}));
  await restored.check();
  expect(restored.getSnapshot()).toMatchObject({phase: "unknown", recordedCommitted: true, status: {type: "noSqlReceipt"}});
  expect(archiveRecoveryMessage(restored.getSnapshot())).toContain("本机已记录原操作提交");
  await restored.retryOriginal();
  expect(f.setArchived).toHaveBeenCalledTimes(1);
  expect(readConversationArchive(f.storage, id)).toEqual(saved);
  await restored.check();
  expect(restored.getSnapshot()).toMatchObject({phase: "committed", recordedCommitted: true, proof: proof(saved.intent)});
  await restored.readCurrent();
  expect(f.setArchived).toHaveBeenCalledTimes(1);
  expect(readConversationArchive(f.storage, id)).toBeNull();
});

it("refuses a stale owner's attempts to downgrade or remove another owner's recorded commit", async () => {
  const f = fixture(); f.setArchived.mockRejectedValueOnce(new Error("lost reply"));
  await f.controller.submit(summary, context);
  const original = readConversationArchive(f.storage, id)!;
  const stale = new ConversationArchiveController(id, f.ports, f.storage);
  saveConversationArchive(f.storage, {...original, stage: "committed"}, false);
  f.archiveStatus.mockImplementationOnce(async intent => ({schemaVersion: 1, operationId: intent.operationId, conversationId: intent.id,
    status: {type: "noSqlReceipt", epoch: "33333333-3333-4333-8333-333333333333", sqlFence: "0", closed: false}}));
  await stale.check(); await stale.retryOriginal();
  expect(stale.getSnapshot()).toMatchObject({phase: "localError", localIssue: expect.objectContaining({message: "ARCHIVE_RECOVERY_CONFLICT"})});
  expect(f.setArchived).toHaveBeenCalledTimes(1);
  expect(readConversationArchive(f.storage, id)).toEqual({...original, stage: "committed"});
  expect(() => clearConversationArchive(f.storage, original, false)).toThrow("ARCHIVE_RECOVERY_CONFLICT");
  expect(readConversationArchive(f.storage, id)).toEqual({...original, stage: "committed"});
});

it("requires reliable persistence ACK before dispatch; sticky failure retry preserves the original intent", async () => {
  const f = fixture(); let error: Error | undefined = new Error("private storage detail");
  const flush = vi.fn(async () => { if (error) throw error; });
  const retry = vi.fn(async () => { error = undefined; });
  registerStoragePersistence(f.storage, { backend: "IndexedDB", pending: false, resetting: false, get error() { return error; }, flush, retry,
    reset: vi.fn(), subscribe: vi.fn(() => () => {}), close: vi.fn() } satisfies StoragePersistence);
  await f.controller.submit(summary, context); const intent = f.controller.getSnapshot().intent!;
  expect(f.setArchived).not.toHaveBeenCalled(); expect(f.controller.getSnapshot().phase).toBe("localError");
  expect(archiveRecoveryMessage(f.controller.getSnapshot())).not.toContain("private storage detail");
  await f.controller.retryPersistence(); expect(retry).toHaveBeenCalledOnce(); expect(f.setArchived).toHaveBeenCalledExactlyOnceWith(intent);
  expect(flush.mock.invocationCallOrder[1]).toBeLessThan(f.setArchived.mock.invocationCallOrder[0]!);
});

it("does not redispatch after a committed-stage or cleanup flush failure", async () => {
  const f = fixture(); const flush = vi.fn().mockResolvedValueOnce(undefined).mockRejectedValueOnce(new Error("quota")).mockResolvedValue(undefined);
  registerStoragePersistence(f.storage, { backend: "IndexedDB", pending: false, resetting: false, error: undefined, flush, retry: vi.fn(),
    reset: vi.fn(), subscribe: vi.fn(() => () => {}), close: vi.fn() });
  await f.controller.submit(summary, context); expect(f.controller.getSnapshot()).toMatchObject({ phase: "committed", localIssue: expect.any(Error) });
  expect(f.read).not.toHaveBeenCalled(); await f.controller.retryPersistence(); expect(f.setArchived).toHaveBeenCalledTimes(1);
  flush.mockRejectedValueOnce(new Error("cleanup failed")); await f.controller.readCurrent();
  expect(archiveUnresolved(f.controller.getSnapshot())).toBe(true); await f.controller.submit(summary, context);
  await f.controller.retryPersistence(); expect(archiveUnresolved(f.controller.getSnapshot())).toBe(false);
  expect(f.setArchived).toHaveBeenCalledTimes(1); expect(f.read).toHaveBeenCalledTimes(1);
});

it("retains the owner across unmount and serializes repeated status/read actions", async () => {
  const f = fixture(), pending = deferred<AgentArchiveProof>(); f.setArchived.mockReturnValueOnce(pending.promise);
  const lease = acquireConversationArchiveOwner(id, f.ports, f.storage);
  const work = lease.controller.submit(summary, context); await vi.waitFor(() => expect(f.setArchived).toHaveBeenCalled()); lease.release();
  const next = acquireConversationArchiveOwner(id, f.ports, f.storage); expect(next.controller).toBe(lease.controller);
  pending.resolve(proof(f.setArchived.mock.calls[0]![0])); await work; next.release();
  expect(f.setArchived).toHaveBeenCalledOnce();
});

it("preserves a foreign locator and rejects late current metadata adoption", async () => {
  const f = fixture(), pending = deferred<AgentConversationSummary>(); f.read.mockReturnValueOnce(pending.promise);
  const work = f.controller.submit(summary, context); await vi.waitFor(() => expect(f.read).toHaveBeenCalled());
  const original = readConversationArchive(f.storage, id)!;
  f.storage.removeItem(`research-assistant:conversation-archive:v1:${id}`);
  const foreign = { ...original, intent: { ...original.intent, operationId: crypto.randomUUID() } };
  saveConversationArchive(f.storage, foreign, true);
  pending.resolve({ ...summary, archived: true }); expect(await work).toBeNull();
  expect(readConversationArchive(f.storage, id)).toEqual(foreign);
  expect(f.controller.getSnapshot().localIssue).toMatchObject({ message: "ARCHIVE_RECOVERY_CONFLICT" });
});

it("discovers an absent row only in its exact host scope, and preserves malformed records", async () => {
  const f = fixture(); f.setArchived.mockRejectedValueOnce("unavailable"); await f.controller.submit(summary, context);
  expect(discoverConversationArchives(f.storage, context).map(record => record.intent.id)).toEqual([id]);
  expect(discoverConversationArchives(f.storage, { ...context, binding: { ...context.binding, resourceId: "casesensitivepaper" } })).toEqual([]);
  const name = `research-assistant:conversation-archive:v1:${id}`; f.storage.setItem(name, "malformed");
  const restored = new ConversationArchiveController(id, f.ports, f.storage);
  await restored.submit(summary, context); expect(f.setArchived).toHaveBeenCalledTimes(1);
  expect(restored.getSnapshot().localIssue).toMatchObject({ message: "ARCHIVE_RECOVERY_INVALID" }); expect(f.storage.getItem(name)).toBe("malformed");
});

it("reports a malformed locator while still discovering other original operations", async () => {
  const f = fixture(); f.storage.setItem("research-assistant:conversation-archive:v1:33333333-3333-4333-8333-333333333333", "malformed");
  f.setArchived.mockRejectedValueOnce("unavailable"); await f.controller.submit(summary, context);
  const report = vi.fn(); expect(discoverConversationArchives(f.storage, context, report).map(record => record.intent.id)).toEqual([id]);
  expect(report).toHaveBeenCalledExactlyOnceWith(expect.objectContaining({message: "ARCHIVE_RECOVERY_INVALID"}));
});

it("cannot resurrect a cleared locator when an old mutation returns late", async () => {
  const f = fixture(), pending = deferred<AgentArchiveProof>(); f.setArchived.mockReturnValueOnce(pending.promise);
  const work = f.controller.submit(summary, context); await vi.waitFor(() => expect(f.setArchived).toHaveBeenCalled());
  f.storage.clear(); pending.resolve(proof(f.setArchived.mock.calls[0]![0])); expect(await work).toBeNull();
  expect(f.storage.length).toBe(0); expect(f.read).not.toHaveBeenCalled();
  expect(f.controller.getSnapshot()).toMatchObject({phase: "committed", localIssue: expect.objectContaining({message: "ARCHIVE_RECOVERY_CONFLICT"})});
});

it("keeps blocked Unconfirmed inspection read-only and rejects proof identity mismatch", async () => {
  const f = fixture(); f.setArchived.mockImplementationOnce(async intent => ({...proof(intent), operationId: crypto.randomUUID()}));
  await f.controller.submit(summary, context); expect(f.controller.getSnapshot().phase).toBe("unknown"); expect(f.read).not.toHaveBeenCalled();
  f.archiveStatus.mockImplementation(async intent => ({schemaVersion: 1, operationId: intent.operationId, conversationId: intent.id,
    status: {type: "unconfirmed", epoch: "22222222-2222-4222-8222-222222222222", sqlFence: "8", closed: false, blocked: true, reason: "postCommitUnknown"}}));
  await f.controller.check(); await f.controller.retryOriginal(); expect(f.controller.getSnapshot().phase).toBe("unknown");
  expect(f.setArchived).toHaveBeenCalledTimes(1); expect(f.read).not.toHaveBeenCalled();
});

it("does not invent a history watermark for invalid fresh archive input", async () => {
  const f = fixture(); await f.controller.submit({...summary, durableCount: Number.MAX_SAFE_INTEGER + 1}, context);
  expect(f.setArchived).not.toHaveBeenCalled(); expect(f.read).not.toHaveBeenCalled(); expect(f.storage.length).toBe(0);
  expect(f.controller.getSnapshot()).toMatchObject({phase: "rejected", cause: "invalidInput"});
});
