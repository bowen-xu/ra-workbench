// @vitest-environment happy-dom
import { expect, it, vi } from "vitest";
import { registerAppStoragePrefix, selectAppStorage } from "../../runtime/appStorage";
import { clearConversationDeletion, discoverConversationDeletions, readConversationDeletion, saveConversationDeletion,
  subscribeConversationDeletions, type ConversationDeletionRecord } from "./conversationDeletionPending";
const id = "00000000-0000-4000-8000-000000000001", operation = "00000000-0000-4000-8000-000000000002", epoch = "00000000-0000-4000-8000-000000000003";
const record: ConversationDeletionRecord = {schemaVersion: 1, intent: {schemaVersion: 1, requestId: operation, conversationId: id, binding: null,
  expectedTitle: "Original", expectedDurableCount: 2, force: false}, internal: false, epoch, stage: "unknown", reason: null, receipt: null};
function memory(): Storage {
  const values = new Map<string, string>();
  return {get length() {return values.size;}, key: index => [...values.keys()][index] ?? null, getItem: key => values.get(key) ?? null,
    setItem: (key, value) => {values.set(key, value);}, removeItem: key => {values.delete(key);}, clear: () => values.clear()};
}
it("preserves complete confirmation and refuses changed IDs, epoch, scope or force on an uncertain operation", () => {
  const storage = memory(); saveConversationDeletion(storage, record, true);
  expect(readConversationDeletion(storage, id)).toEqual(record);
  for (const patch of [{intent: {...record.intent, requestId: crypto.randomUUID()}}, {intent: {...record.intent, expectedTitle: "Later"}},
    {intent: {...record.intent, expectedDurableCount: 3}}, {intent: {...record.intent, force: true}}, {internal: true}, {epoch: crypto.randomUUID()},
    {epoch: null, stage: "pending" as const}, {stage: "pending" as const}]) {
    expect(() => saveConversationDeletion(storage, {...record, ...patch})).toThrow();
  }
  expect(() => clearConversationDeletion(storage, record)).toThrow(); expect(readConversationDeletion(storage, id)).toEqual(record);
});
it("allows force with the same identity only after exact authoritative ordinary retention", () => {
  const storage = memory(), retained = {...record, stage: "retained" as const, reason: "executionWorkspace" as const};
  saveConversationDeletion(storage, retained, true);
  const force = {...retained, intent: {...retained.intent, force: true}, stage: "pending" as const, reason: null};
  saveConversationDeletion(storage, force); expect(readConversationDeletion(storage, id)).toEqual(force);
  expect(() => saveConversationDeletion(storage, retained)).toThrow();
  expect(() => clearConversationDeletion(storage, retained)).toThrow();
});
it("never downgrades a committed operation or clears another request", () => {
  const storage = memory(), committed: ConversationDeletionRecord = {...record, stage: "committed",
    receipt: {schemaVersion: 1, requestId: operation, conversationId: id}};
  saveConversationDeletion(storage, committed, true);
  expect(() => saveConversationDeletion(storage, record)).toThrow();
  expect(() => clearConversationDeletion(storage, {...committed, intent: {...record.intent, expectedTitle: "Changed"}})).toThrow();
  clearConversationDeletion(storage, committed); expect(readConversationDeletion(storage, id)).toBeNull();
});
it("discovers missing rows by exact original host and internal classification without backend I/O", () => {
  const storage = memory(), binding = {kind: "note" as const, resourceId: "original"};
  const bound = {...record, intent: {...record.intent, binding}, internal: true};
  saveConversationDeletion(storage, bound, true);
  expect(discoverConversationDeletions(storage, binding, true)).toEqual([bound]);
  expect(discoverConversationDeletions(storage, binding, false)).toEqual([]);
  expect(discoverConversationDeletions(storage, null, true)).toEqual([]);
  expect(discoverConversationDeletions(storage, {...binding, resourceId: "other"}, true)).toEqual([]);
});
it("retains malformed evidence and reports it rather than treating it as an empty slot", () => {
  const storage = memory(); saveConversationDeletion(storage, record, true); const key = storage.key(0)!;
  const malformed = JSON.stringify({...record, stage: "committed", receipt: {schemaVersion: 1, requestId: operation, conversationId: epoch}});
  storage.setItem(key, malformed);
  const issue = vi.fn(); expect(discoverConversationDeletions(storage, null, false, issue)).toEqual([]);
  expect(issue).toHaveBeenCalledOnce(); expect(storage.getItem(key)).toBe(malformed);
  expect(() => saveConversationDeletion(storage, record, true)).toThrow();
});
it("scopes recovery invalidations to the captured storage even after runtime selection changes", () => {
  const storage = memory(), other = memory(), changed = vi.fn();
  registerAppStoragePrefix(storage, "original:"); registerAppStoragePrefix(other, "other:");
  selectAppStorage(other); const stop = subscribeConversationDeletions(storage, changed);
  try {
    saveConversationDeletion(other, record, true); expect(changed).not.toHaveBeenCalled();
    window.dispatchEvent(new StorageEvent("storage", {key: "other:research-assistant:conversation-deletion:v1:example"}));
    expect(changed).not.toHaveBeenCalled();
    saveConversationDeletion(storage, record, true); expect(changed).toHaveBeenCalledOnce();
    window.dispatchEvent(new StorageEvent("storage", {key: "original:research-assistant:conversation-deletion:v1:example"}));
    expect(changed).toHaveBeenCalledTimes(2);
  } finally {stop(); selectAppStorage(memory());}
});
