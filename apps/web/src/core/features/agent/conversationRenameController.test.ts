import { expect, it, vi } from "vitest";
import type { AgentRenameRequest, AgentRenameStatusResponse } from "@research-assistant/core/agent";
import { ConversationRenameController, acquireConversationRenameOwner, resetConversationRenameOwnersForTests } from "./conversationRenameController";
import { nameInitialConversation } from "./automaticConversationName";
import { conversationRenameMessage } from "./ConversationRenameRecovery";
import { renameConversationId as id, renameEpoch as epoch, renameProof, renameSummary } from "./conversationRenameTestFacts";

function fixture() {
  const values = new Map<string, string>();
  const storage: Storage = { get length() { return values.size; }, clear: () => values.clear(), key: n => [...values.keys()][n] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); } };
  const catalog = { rename: vi.fn(async (request: AgentRenameRequest) => renameProof(request)),
    renameStatus: vi.fn(async (request: AgentRenameRequest): Promise<AgentRenameStatusResponse> => ({ schemaVersion: 1,
      operationId: request.operationId, conversationId: request.id, status: { type: "committed", proof: renameProof(request) } })) };
  const metadata = { read: vi.fn(async () => renameSummary) }, ports = { catalog, metadata };
  return { storage, values, ports, catalog, metadata, controller: new ConversationRenameController(id, ports, storage) };
}
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; }); return { promise, resolve, reject }; }

it("returns current metadata after immutable proof, including a newer name than the old target", async () => {
  const f = fixture(), newer = { ...renameSummary, title: "后来手动修改的名称" };
  f.metadata.read.mockResolvedValueOnce(newer);
  expect(await f.controller.submit("原名称", "更新名称")).toEqual(newer);
  expect(f.catalog.rename).toHaveBeenCalledExactlyOnceWith({ operationId: expect.any(String), id, expectedTitle: "原名称", title: "更新名称" });
  expect(f.controller.getSnapshot()).toMatchObject({ phase: "ready", current: newer, proof: { sqlFence: "9007199254740993" } });
  expect(f.values.size).toBe(0);
});

it("does not repeat a committed mutation after current metadata fails to read", async () => {
  const f = fixture(); f.metadata.read.mockRejectedValueOnce(new Error("read unavailable"));
  expect(await f.controller.submit("原名称", "更新名称")).toBeNull();
  expect(f.controller.getSnapshot().phase).toBe("readError");
  expect(await f.controller.submit("原名称", "另一个名称")).toBeNull();
  expect(await f.controller.readCurrent()).toEqual(renameSummary);
  expect(f.catalog.rename).toHaveBeenCalledTimes(1);
  expect(f.metadata.read).toHaveBeenCalledTimes(2);
});

it("locks an unknown intent and inspects its original fields without hidden metadata reads", async () => {
  const f = fixture(); f.catalog.rename.mockRejectedValueOnce({ code: "storage", reason: { code: "DATA_SQL_COMMIT_UNKNOWN" } });
  await f.controller.submit("原名称", "更新名称");
  const intent = f.controller.getSnapshot().intent!;
  expect(await f.controller.submit("原名称", "改过的输入")).toBeNull();
  expect(f.controller.dismiss()).toBe(false);
  f.catalog.renameStatus.mockResolvedValueOnce({ schemaVersion: 1, operationId: intent.operationId, conversationId: id,
    status: { type: "unconfirmed", epoch, sqlFence: "9007199254740993", blocked: true, closed: false, reason: "owner-blocked" } });
  await f.controller.check();
  expect(f.controller.getSnapshot().phase).toBe("unknown");
  await f.controller.check();
  expect(f.controller.getSnapshot().phase).toBe("committed");
  expect(f.catalog.renameStatus).toHaveBeenNthCalledWith(1, intent);
  expect(f.catalog.renameStatus).toHaveBeenNthCalledWith(2, intent);
  expect(f.catalog.rename).toHaveBeenCalledTimes(1);
  expect(f.metadata.read).not.toHaveBeenCalled();
  expect(await f.controller.readCurrent()).toEqual(renameSummary);
});

it("retries a clean missing receipt only after explicit action using the same operation UUID", async () => {
  const f = fixture(); f.catalog.rename.mockRejectedValueOnce(new Error("reply lost"));
  await f.controller.submit("原名称", "更新名称");
  const intent = f.controller.getSnapshot().intent!;
  f.catalog.renameStatus.mockResolvedValueOnce({ schemaVersion: 1, operationId: intent.operationId, conversationId: id,
    status: { type: "noSqlReceipt", epoch, sqlFence: "0", closed: false } });
  await f.controller.check();
  expect(f.catalog.rename).toHaveBeenCalledTimes(1);
  expect(f.metadata.read).not.toHaveBeenCalled();
  expect(await f.controller.retryOriginal()).toEqual(renameSummary);
  expect(f.catalog.rename.mock.calls[0]).toEqual(f.catalog.rename.mock.calls[1]);
});

it("reopens a persisted operation for status only and retains the owner through view detachment", async () => {
  const f = fixture(); f.catalog.rename.mockRejectedValueOnce(new Error("lost"));
  const view = acquireConversationRenameOwner(id, f.ports, f.storage);
  await view.controller.submit("原名称", "更新名称");
  const intent = view.controller.getSnapshot().intent;
  view.release();
  const returned = acquireConversationRenameOwner(id, f.ports, f.storage);
  expect(returned.controller).toBe(view.controller);
  returned.release(); resetConversationRenameOwnersForTests(f.storage);
  const reopened = acquireConversationRenameOwner(id, f.ports, f.storage);
  expect(reopened.controller.getSnapshot()).toMatchObject({ phase: "unknown", intent });
  expect(f.catalog.renameStatus).not.toHaveBeenCalled();
  await reopened.controller.check();
  expect(f.catalog.renameStatus).toHaveBeenCalledExactlyOnceWith(intent);
  expect(f.catalog.rename).toHaveBeenCalledTimes(1);
  expect(f.metadata.read).not.toHaveBeenCalled();
  reopened.release(); resetConversationRenameOwnersForTests(f.storage);
});

it("assigns automatic naming a persistent phase UUID distinct from the accepted model request", async () => {
  const f = fixture(), modelRequest = crypto.randomUUID();
  expect(await nameInitialConversation(id, modelRequest, "首条消息", f.ports, f.storage)).toEqual(renameSummary);
  const intent = f.catalog.rename.mock.calls[0]![0];
  expect(intent.operationId).not.toBe(modelRequest);
  f.metadata.read.mockResolvedValueOnce({ ...renameSummary, title: "更新后又人工修改" });
  await nameInitialConversation(id, modelRequest, "新的草稿不应覆盖原意图", f.ports, f.storage);
  expect(f.catalog.rename.mock.calls[1]![0]).toEqual(intent);
  expect(f.metadata.read).toHaveBeenCalledTimes(2);
  resetConversationRenameOwnersForTests(f.storage);
});

it("preserves automatic naming uncertainty without triggering any other phase", async () => {
  const f = fixture(), modelRequest = crypto.randomUUID();
  f.catalog.rename.mockRejectedValueOnce({ code: "storage", reason: { code: "DATA_SQL_COMMIT_UNKNOWN" } });
  await nameInitialConversation(id, modelRequest, "首条消息", f.ports, f.storage);
  await nameInitialConversation(id, modelRequest, "首条消息", f.ports, f.storage);
  expect(f.catalog.rename).toHaveBeenCalledTimes(1);
  expect(f.catalog.renameStatus).not.toHaveBeenCalled();
  expect(f.metadata.read).not.toHaveBeenCalled();
  resetConversationRenameOwnersForTests(f.storage);
});

it("preserves invalid or foreign automatic phase records without generating another operation", async () => {
  const f = fixture(), sourceRequestId = crypto.randomUUID();
  const key = `research-assistant:initial-conversation-name:v1:${id}:${sourceRequestId}`;
  const intent = { operationId: crypto.randomUUID(), id, expectedTitle: "新对话", title: "原冻结名称" };
  const records = ["{", JSON.stringify({ schemaVersion: 2, sourceRequestId, intent }),
    JSON.stringify({ schemaVersion: 1, sourceRequestId: crypto.randomUUID(), intent }),
    JSON.stringify({ schemaVersion: 1, sourceRequestId, intent: null }),
    JSON.stringify({ schemaVersion: 1, sourceRequestId, intent: { ...intent, id: crypto.randomUUID() } })];
  for (const raw of records) {
    f.storage.setItem(key, raw);
    await expect(nameInitialConversation(id, sourceRequestId, "新草稿", f.ports, f.storage)).rejects.toBeDefined();
    expect(f.storage.getItem(key)).toBe(raw);
  }
  expect(f.catalog.rename).not.toHaveBeenCalled();
  expect(f.catalog.renameStatus).not.toHaveBeenCalled();
  expect(f.metadata.read).not.toHaveBeenCalled();
  resetConversationRenameOwnersForTests(f.storage);
});

it("reopens a committed operation with a failed view read for status rather than another mutation", async () => {
  const f = fixture(); f.metadata.read.mockRejectedValueOnce("unavailable");
  await f.controller.submit("原名称", "更新名称");
  const intent = f.controller.getSnapshot().intent;
  const reopened = new ConversationRenameController(id, f.ports, f.storage);
  expect(reopened.getSnapshot()).toMatchObject({ phase: "unknown", intent });
  await reopened.check();
  expect(reopened.getSnapshot().phase).toBe("committed");
  expect(f.catalog.rename).toHaveBeenCalledTimes(1);
  expect(f.metadata.read).toHaveBeenCalledTimes(1);
  await reopened.readCurrent();
  expect(f.metadata.read).toHaveBeenCalledTimes(2);
  expect(f.catalog.rename).toHaveBeenCalledTimes(1);
});

it("does not write or read current metadata from a closed missing-receipt status", async () => {
  const f = fixture(); f.catalog.rename.mockRejectedValueOnce(new Error("lost reply"));
  await f.controller.submit("原名称", "更新名称");
  const intent = f.controller.getSnapshot().intent!;
  f.catalog.renameStatus.mockResolvedValueOnce({ schemaVersion: 1, operationId: intent.operationId, conversationId: id,
    status: { type: "noSqlReceipt", epoch, sqlFence: "0", closed: true } });
  await f.controller.check();
  expect(await f.controller.retryOriginal()).toBeNull();
  expect(f.controller.dismiss()).toBe(false);
  expect(f.catalog.rename).toHaveBeenCalledTimes(1);
  expect(f.metadata.read).not.toHaveBeenCalled();
});

it("keeps safe SQL diagnostics and hides arbitrary transport or provider payloads", () => {
  expect(conversationRenameMessage(new Error("PRIVATE_PROVIDER_BODY"))).not.toContain("PRIVATE_PROVIDER_BODY");
  const safe = conversationRenameMessage({ code: "storage", reason: { code: "DATA_SQL_COMMIT_UNKNOWN",
    message: "PRIVATE_SQL_BODY", reason: { operationId: id, epoch, sqlFence: "9007199254740993", phase: "commit", safeCause: "commit-ack-lost" } } });
  expect(safe).toContain("DATA_SQL_COMMIT_UNKNOWN");
  expect(safe).toContain("9007199254740993");
  expect(safe).toContain(id);
  expect(safe).not.toContain("PRIVATE_SQL_BODY");
});

it("retains safe transport code and valid HTTP status without arbitrary error messages", () => {
  const text = conversationRenameMessage({ code: "METADATA_READ_UNAVAILABLE", status: 503, message: "PRIVATE_PROVIDER_BODY" });
  expect(text).toContain("METADATA_READ_UNAVAILABLE"); expect(text).toContain("HTTP 503");
  expect(text).not.toContain("PRIVATE_PROVIDER_BODY");
  expect(conversationRenameMessage({ code: "private body with spaces", status: 999, message: "PRIVATE_PROVIDER_BODY" })).not.toMatch(/private|999|PRIVATE_PROVIDER_BODY/);
});

it("keeps a confirmed proof when persisting its committed locator fails", async () => {
  const f = fixture(), set = f.storage.setItem.bind(f.storage);
  f.storage.setItem = vi.fn().mockImplementationOnce(set).mockImplementation(() => { throw new Error("quota"); });
  expect(await f.controller.submit("原名称", "更新名称")).toBeNull();
  expect(f.controller.getSnapshot()).toMatchObject({ phase: "committed", proof: { conversationId: id } });
  expect(f.metadata.read).not.toHaveBeenCalled();
  expect(await f.controller.submit("原名称", "另一个名称")).toBeNull();
  expect(await f.controller.readCurrent()).toEqual(renameSummary);
  expect(f.catalog.rename).toHaveBeenCalledTimes(1);
});

it("keeps status-confirmed proof through local persistence failure without a hidden view read", async () => {
  const f = fixture(), set = f.storage.setItem.bind(f.storage);
  f.storage.setItem = vi.fn().mockImplementationOnce(set).mockImplementation(() => { throw new Error("quota"); });
  f.catalog.rename.mockRejectedValueOnce(new Error("lost reply"));
  await f.controller.submit("原名称", "更新名称");
  await f.controller.check();
  expect(f.controller.getSnapshot()).toMatchObject({ phase: "committed", proof: { conversationId: id } });
  expect(f.metadata.read).not.toHaveBeenCalled();
  expect(f.catalog.rename).toHaveBeenCalledTimes(1);
  expect(await f.controller.readCurrent()).toEqual(renameSummary);
});

it.each(["proof", "domain", "metadata"])("does not replace or delete a foreign locator after a late %s result", async boundary => {
  const f = fixture(), proofReply = deferred<ReturnType<typeof renameProof>>(), readReply = deferred<typeof renameSummary>();
  if (boundary === "metadata") f.metadata.read.mockReturnValueOnce(readReply.promise);
  else f.catalog.rename.mockReturnValueOnce(proofReply.promise);
  const operation = f.controller.submit("原名称", "更新名称");
  const original = f.controller.getSnapshot().intent!;
  if (boundary === "metadata") await vi.waitFor(() => expect(f.metadata.read).toHaveBeenCalledTimes(1));
  const locator = [...f.values.keys()][0]!;
  const foreign = JSON.stringify({ schemaVersion: 1, intent: { ...original, operationId: crypto.randomUUID(), title: "另一窗口名称" }, stage: "unknown" });
  f.storage.setItem(locator, foreign);
  if (boundary === "proof") proofReply.resolve(renameProof(original));
  else if (boundary === "domain") proofReply.reject("conflict");
  else readReply.resolve(renameSummary);
  await operation;
  expect(f.storage.getItem(locator)).toBe(foreign);
  expect(f.controller.getSnapshot().localIssue).toMatchObject({ message: "RENAME_RECOVERY_CONFLICT" });
  if (boundary === "proof") {
    expect(f.controller.getSnapshot()).toMatchObject({ phase: "committed", proof: { operationId: original.operationId } });
    expect(f.metadata.read).not.toHaveBeenCalled();
  } else if (boundary === "metadata") expect(f.controller.getSnapshot()).toMatchObject({ phase: "ready", proof: { operationId: original.operationId } });
  else expect(f.controller.getSnapshot()).toMatchObject({ phase: "rejected", cause: "conflict" });
});
