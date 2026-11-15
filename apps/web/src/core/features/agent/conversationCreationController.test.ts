import { expect, it, vi } from "vitest";
import type { AgentCreateBoundRequest, AgentCreateBoundStatusResponse } from "@research-assistant/core/agent";
import { ConversationCreationController, acquireConversationCreationOwner, resetConversationCreationOwnersForTests } from "./conversationCreationController";
import { pendingBoundConversationId, readBoundCreation, reserveBoundConversation } from "./boundConversationCreationPending";
import { creationConversationId as id, creationHostId as hostId, creationEpoch as epoch, creationProof, creationSummary } from "./conversationCreationTestFacts";

function fixture() {
  const values = new Map<string, string>(), binding = { kind: "manuscript" as const, resourceId: hostId };
  const storage: Storage = { get length() { return values.size; }, key: n => [...values.keys()][n] ?? null,
    getItem: key => values.get(key) ?? null, setItem: (key, value) => { values.set(key, value); }, removeItem: key => { values.delete(key); }, clear: () => values.clear() };
  const creation = { createBound: vi.fn(async (intent: AgentCreateBoundRequest) => creationProof(intent)),
    createBoundStatus: vi.fn(async (intent: AgentCreateBoundRequest): Promise<AgentCreateBoundStatusResponse> => ({ schemaVersion: 1,
      operationId: intent.operationId, conversationId: intent.id, status: { type: "committed", proof: creationProof(intent) } })) };
  const metadata = { read: vi.fn(async () => creationSummary) }, ports = { creation, metadata };
  return { values, storage, binding, creation, metadata, ports, controller: new ConversationCreationController(binding, ports, storage) };
}
function deferred<T>() { let resolve!: (value: T) => void; let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((done, fail) => { resolve = done; reject = fail; }); return { promise, resolve, reject }; }

it("reserves a stable pending C and independent creation operation across remounts", () => {
  const f = fixture(); expect(reserveBoundConversation(f.binding, id, f.storage)).toBe(id);
  const saved = readBoundCreation(f.binding, f.storage)!;
  expect(saved.intent.operationId).not.toBe(id); expect(saved.intent.operationId).not.toBe(hostId);
  expect(reserveBoundConversation(f.binding, undefined, f.storage)).toBe(id);
  expect(pendingBoundConversationId(f.binding, f.storage)).toBe(id);
  expect(readBoundCreation(f.binding, f.storage)).toEqual(saved);
  expect(f.creation.createBound).not.toHaveBeenCalled();
});

it("reads current metadata with the original binding and retains the locator until host adoption", async () => {
  const f = fixture();
  expect(await f.controller.submit(id)).toEqual(creationSummary);
  expect(f.metadata.read).toHaveBeenCalledExactlyOnceWith({ id, binding: f.binding });
  expect(f.controller.getSnapshot()).toMatchObject({ phase: "ready", proof: { sqlFence: "9007199254740993" }, current: creationSummary });
  expect(readBoundCreation(f.binding, f.storage)?.stage).toBe("committed");
  const renameKey = `research-assistant:conversation-rename:v1:${id}`;
  f.storage.setItem(renameKey, "rename recovery remains owned by rename");
  expect(f.controller.consumeAdoption(() => true)).toBe(true);
  expect(readBoundCreation(f.binding, f.storage)).toBeNull(); expect(f.storage.getItem(renameKey)).not.toBeNull();
});

it("locks Unknown to the original intent and offers status without hidden read or write", async () => {
  const f = fixture(); f.creation.createBound.mockRejectedValueOnce({ code: "storage", reason: { code: "DATA_SQL_COMMIT_UNKNOWN" } });
  await f.controller.submit(id); const intent = f.controller.getSnapshot().intent!;
  expect(await f.controller.submit(crypto.randomUUID())).toBeNull();
  const reopened = new ConversationCreationController(f.binding, f.ports, f.storage);
  expect(reopened.getSnapshot()).toMatchObject({ phase: "unknown", intent });
  expect(f.creation.createBoundStatus).not.toHaveBeenCalled();
  await reopened.check();
  expect(f.creation.createBoundStatus).toHaveBeenCalledExactlyOnceWith(intent);
  expect(reopened.getSnapshot().phase).toBe("committed");
  expect(f.creation.createBound).toHaveBeenCalledTimes(1); expect(f.metadata.read).not.toHaveBeenCalled();
  await reopened.readCurrent(); expect(f.metadata.read).toHaveBeenCalledTimes(1);
});

it("retries only a clean missing receipt explicitly with the same operation, C and binding", async () => {
  const f = fixture(); f.creation.createBound.mockRejectedValueOnce(new Error("lost reply"));
  await f.controller.submit(id); const intent = f.controller.getSnapshot().intent!;
  f.creation.createBoundStatus.mockResolvedValueOnce({ schemaVersion: 1, operationId: intent.operationId, conversationId: id,
    status: { type: "noSqlReceipt", epoch, sqlFence: "0", closed: false } });
  await f.controller.check(); expect(f.creation.createBound).toHaveBeenCalledTimes(1); expect(f.metadata.read).not.toHaveBeenCalled();
  await f.controller.retryOriginal(); expect(f.creation.createBound.mock.calls[0]).toEqual(f.creation.createBound.mock.calls[1]);
  expect(f.metadata.read).toHaveBeenCalledTimes(1);
});

it("keeps a confirmed proof when its view read fails and retries only that read", async () => {
  const f = fixture(); f.metadata.read.mockRejectedValueOnce("notFound");
  expect(await f.controller.submit(id)).toBeNull(); expect(f.controller.getSnapshot().phase).toBe("readError");
  expect(await f.controller.submit(id)).toBeNull();
  await f.controller.readCurrent(); expect(f.creation.createBound).toHaveBeenCalledTimes(1); expect(f.metadata.read).toHaveBeenCalledTimes(2);
});

it("does not adopt an archived current conversation merely because historical creation committed", async () => {
  const f = fixture(); f.metadata.read.mockResolvedValueOnce({ ...creationSummary, archived: true });
  expect((await f.controller.submit(id))?.archived).toBe(true); expect(f.controller.consumeAdoption(() => true)).toBe(false);
  expect(readBoundCreation(f.binding, f.storage)).not.toBeNull();
  f.metadata.read.mockResolvedValueOnce(creationSummary); await f.controller.readCurrent(); expect(f.controller.consumeAdoption(() => true)).toBe(true);
});

it.each([true, false])("preserves native proof even if local committed persistence fails (status=%s)", async status => {
  const f = fixture(), set = f.storage.setItem.bind(f.storage);
  if (status) { f.creation.createBound.mockRejectedValueOnce("unavailable"); await f.controller.submit(id); }
  f.storage.setItem = vi.fn((key, value) => { if (JSON.parse(value).stage === "committed") throw new Error("quota"); set(key, value); });
  if (status) await f.controller.check(); else await f.controller.submit(id);
  expect(f.controller.getSnapshot()).toMatchObject({ phase: "committed", proof: { conversationId: id }, localIssue: expect.any(Error) });
  expect(f.metadata.read).not.toHaveBeenCalled();
  await f.controller.readCurrent(); expect(f.creation.createBound).toHaveBeenCalledTimes(1);
  expect(f.controller.consumeAdoption(() => true)).toBe(true);
});

it("does not overwrite or clear a foreign locator after a delayed native proof", async () => {
  const f = fixture(), reply = deferred<ReturnType<typeof creationProof>>(); f.creation.createBound.mockReturnValueOnce(reply.promise);
  const pending = f.controller.submit(id), original = f.controller.getSnapshot().intent!, locator = [...f.values.keys()][0]!;
  const foreign = JSON.stringify({ schemaVersion: 1, stage: "unknown", intent: { ...original, operationId: crypto.randomUUID(), id: crypto.randomUUID() } });
  f.storage.setItem(locator, foreign); reply.resolve(creationProof(original)); await pending;
  expect(f.controller.getSnapshot()).toMatchObject({ phase: "committed", proof: { operationId: original.operationId } });
  expect(f.metadata.read).not.toHaveBeenCalled(); expect(f.storage.getItem(locator)).toBe(foreign);
  await f.controller.readCurrent(); expect(f.controller.consumeAdoption(() => true)).toBe(false); expect(f.storage.getItem(locator)).toBe(foreign);
});

it("keeps the operation owner after its initiating view detaches", async () => {
  const f = fixture(), reply = deferred<ReturnType<typeof creationProof>>(); f.creation.createBound.mockReturnValueOnce(reply.promise);
  const lease = acquireConversationCreationOwner(f.binding, f.ports, f.storage), pending = lease.controller.submit(id);
  const intent = lease.controller.getSnapshot().intent!; lease.release(); reply.resolve(creationProof(intent)); await pending;
  const returned = acquireConversationCreationOwner(f.binding, f.ports, f.storage);
  expect(returned.controller).toBe(lease.controller); expect(returned.controller.getSnapshot().phase).toBe("ready");
  returned.release(); resetConversationCreationOwnersForTests(f.storage);
});

it("does not replace invalid, foreign-binding or mismatched pending records", async () => {
  const f = fixture(); reserveBoundConversation(f.binding, id, f.storage); const locator = [...f.values.keys()][0]!;
  const record = JSON.parse(f.storage.getItem(locator)!);
  for (const raw of ["{", JSON.stringify({ ...record, schemaVersion: 2 }), JSON.stringify({ ...record, intent: { ...record.intent, binding: { ...f.binding, resourceId: crypto.randomUUID() } } })]) {
    f.storage.setItem(locator, raw); const owned = new ConversationCreationController(f.binding, f.ports, f.storage);
    expect(await owned.submit(id)).toBeNull(); expect(f.storage.getItem(locator)).toBe(raw);
  }
  expect(f.creation.createBound).not.toHaveBeenCalled(); expect(f.metadata.read).not.toHaveBeenCalled();
});

it.each(["archived", "notFound"])("explicitly ends a known creation after %s without deleting server data or changing the draft", async boundary => {
  const f = fixture();
  if (boundary === "archived") f.metadata.read.mockResolvedValueOnce({ ...creationSummary, archived: true });
  else f.metadata.read.mockRejectedValueOnce("notFound");
  await f.controller.submit(id); const original = f.controller.getSnapshot().intent!;
  expect(f.controller.finishKnown()).toBe(true); expect(readBoundCreation(f.binding, f.storage)).toBeNull();
  const next = reserveBoundConversation(f.binding, undefined, f.storage);
  expect(next).not.toBe(original.id); expect(readBoundCreation(f.binding, f.storage)?.intent.operationId).not.toBe(original.operationId);
  expect(f.creation.createBound).toHaveBeenCalledTimes(1); expect(f.creation.createBoundStatus).not.toHaveBeenCalled();
});

it("never finishes Unknown or removes a foreign locator while ending a known creation", async () => {
  const f = fixture(); f.creation.createBound.mockRejectedValueOnce("unavailable");
  await f.controller.submit(id); expect(f.controller.finishKnown()).toBe(false);
  await f.controller.check(); f.metadata.read.mockRejectedValueOnce("notFound"); await f.controller.readCurrent();
  const locator = [...f.values.keys()][0]!, row = JSON.parse(f.storage.getItem(locator)!);
  const foreign = JSON.stringify({ ...row, intent: { ...row.intent, operationId: crypto.randomUUID() } });
  f.storage.setItem(locator, foreign); expect(f.controller.finishKnown()).toBe(false); expect(f.storage.getItem(locator)).toBe(foreign);
});

it.each(["missing", "foreign", "corrupt"])("does not execute host adoption when its locator became %s during the current read", async boundary => {
  const f = fixture(), reply = deferred<typeof creationSummary>(); f.metadata.read.mockReturnValueOnce(reply.promise);
  const pending = f.controller.submit(id); await vi.waitFor(() => expect(f.metadata.read).toHaveBeenCalledOnce());
  const locator = [...f.values.keys()][0]!, row = JSON.parse(f.storage.getItem(locator)!);
  const foreign = boundary === "corrupt" ? "{" : JSON.stringify({ ...row, intent: { ...row.intent, operationId: crypto.randomUUID() } });
  if (boundary === "missing") f.storage.removeItem(locator); else f.storage.setItem(locator, foreign);
  reply.resolve(creationSummary); await pending;
  const adopt = vi.fn(() => true); expect(f.controller.consumeAdoption(adopt)).toBe(false); expect(adopt).not.toHaveBeenCalled();
  expect(f.storage.getItem(locator)).toBe(boundary === "missing" ? null : foreign);
  expect(f.controller.getSnapshot().proof).not.toBeNull();
});

it("retains a successful host adoption when local cleanup fails and retries only that cleanup", async () => {
  const f = fixture(); await f.controller.submit(id);
  const remove = f.storage.removeItem.bind(f.storage); f.storage.removeItem = vi.fn(() => { throw new Error("quota"); });
  const adopt = vi.fn(() => true); expect(f.controller.consumeAdoption(adopt)).toBe(true);
  expect(f.controller.getSnapshot()).toMatchObject({ adopted: true, phase: "ready", proof: { conversationId: id }, localIssue: expect.any(Error) });
  const nextView=vi.fn(()=>false);expect(f.controller.consumeAdoption(nextView)).toBe(false);expect(nextView).toHaveBeenCalledOnce();expect(adopt).toHaveBeenCalledOnce();
  f.storage.removeItem = remove; expect(f.controller.clearAdopted()).toBe(true);
  expect(readBoundCreation(f.binding, f.storage)).toBeNull(); expect(f.creation.createBound).toHaveBeenCalledTimes(1); expect(f.metadata.read).toHaveBeenCalledTimes(1);
});

it("leaves the original locator when the captured host declines adoption", async () => {
  const f = fixture(); await f.controller.submit(id); const saved = readBoundCreation(f.binding, f.storage);
  expect(f.controller.consumeAdoption(() => false)).toBe(false); expect(readBoundCreation(f.binding, f.storage)).toEqual(saved);
  expect(f.controller.getSnapshot().adopted).toBe(false);
});

it("requires a newly attached view to qualify adoption even when the retained owner already adopted once",async()=>{
  const f=fixture(),lease=acquireConversationCreationOwner(f.binding,f.ports,f.storage);await lease.controller.submit(id);
  f.storage.removeItem=vi.fn(()=>{throw new Error("quota");});const first=vi.fn(()=>true);expect(lease.controller.consumeAdoption(first)).toBe(true);lease.release();
  const next=acquireConversationCreationOwner(f.binding,f.ports,f.storage);expect(next.controller).toBe(lease.controller);
  const declined=vi.fn(()=>false);expect(next.controller.consumeAdoption(declined)).toBe(false);expect(declined).toHaveBeenCalledOnce();
  const accepted=vi.fn(()=>true);expect(next.controller.consumeAdoption(accepted)).toBe(true);expect(accepted).toHaveBeenCalledOnce();
  expect(f.creation.createBound).toHaveBeenCalledTimes(1);next.release();resetConversationCreationOwnersForTests(f.storage);
});
