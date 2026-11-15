import { expect, it, vi } from "vitest";
import { createMockAgentRecoveryStorage } from "../../../app/runtime/mock/MockAgentRecoveryStorage";
import { appStorageScopedKey } from "../../runtime/appStorage";
import { registerStoragePersistence } from "../../runtime/storagePersistence";
import { saveConversationDeletionBatch, discoverConversationDeletionBatches } from "./conversationDeletionBatchPending";
import { readConversationDeletion, saveConversationDeletion } from "./conversationDeletionPending";
import { retireLegacyConversationDeletionBatches } from "./retireLegacyConversationDeletionBatches";
import { ConversationDeletionBatchController } from "./conversationDeletionBatchController";
import type { ConversationOperationLock } from "./conversationOperationLock";

const uuid = (n: number) => `00000000-0000-4000-8000-${String(n).padStart(12, "0")}`;
const binding = {kind: "manuscript" as const, resourceId: "synthetic"};
const requests = [1, 2].map(n => ({schemaVersion: 1 as const, requestId: uuid(n + 10), conversationId: uuid(n),
  binding, expectedTitle: `Archive ${n}`, expectedDurableCount: n, force: true}));
const batch = {schemaVersion: 1 as const, batchId: uuid(50), binding, internal: false,
  epoch: null, phase: "authorized" as const, retained: [], requests};
const record = (request: typeof requests[number]) => ({schemaVersion: 1 as const, intent: request, internal: false,
  epoch: null, stage: "pending" as const, reason: null, receipt: null});
function fixture() {
  const storage = createMockAgentRecoveryStorage("retired-clear");
  const held = new Set<string>();
  const lock: ConversationOperationLock = {async run(name, operation) {
    if (held.has(name)) throw Error("DELETION_BATCH_LOCK_BUSY");
    held.add(name); try {return await operation();} finally {held.delete(name);}
  }};
  saveConversationDeletionBatch(storage, batch);
  saveConversationDeletion(storage, record(requests[0]!), true);
  return {storage, lock, held};
}

it("preserves the original batch and matching members before revoking their active authorization", async () => {
  const {storage, lock} = fixture();
  const other = {...requests[1]!, requestId: uuid(70)};
  saveConversationDeletion(storage, record(other), true);
  await retireLegacyConversationDeletionBatches(storage, lock);
  expect(discoverConversationDeletionBatches(storage)).toEqual([]);
  expect(readConversationDeletion(storage, requests[0]!.conversationId)).toBeNull();
  expect(readConversationDeletion(storage, other.conversationId)?.intent).toEqual(other);
  const preserved = storage.getItem(`research-assistant:conversation-deletion-batch-retired:v1:${batch.batchId}`);
  expect(JSON.parse(preserved!)).toEqual({batch, records: [record(requests[0]!) ]});
});

it("leaves the original records intact if their owner holds a member lock", async () => {
  const {storage, lock, held} = fixture();
  held.add(appStorageScopedKey(storage, `research-assistant:conversation-deletion-lock:v1:${requests[0]!.conversationId}`));
  await expect(retireLegacyConversationDeletionBatches(storage, lock)).rejects.toThrow("DELETION_BATCH_LOCK_BUSY");
  expect(discoverConversationDeletionBatches(storage)).toEqual([batch]);
  expect(readConversationDeletion(storage, requests[0]!.conversationId)).toEqual(record(requests[0]!));
});

it("does not revoke the original before its preserved copy is durable", async () => {
  const {storage, lock} = fixture();
  const flush = vi.fn().mockRejectedValueOnce(Error("disk unavailable")).mockResolvedValue(undefined);
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false, flush,
    retry: async () => undefined, assertCurrent: async () => undefined, reset: async () => undefined, subscribe: () => () => undefined, close: () => undefined});
  await expect(retireLegacyConversationDeletionBatches(storage, lock)).rejects.toThrow("disk unavailable");
  expect(discoverConversationDeletionBatches(storage)).toEqual([batch]);
  expect(readConversationDeletion(storage, requests[0]!.conversationId)).toEqual(record(requests[0]!));
  await retireLegacyConversationDeletionBatches(storage, lock);
  expect(discoverConversationDeletionBatches(storage)).toEqual([]);
});

it("keeps the old index blocked if durable revocation fails and permits a later retry", async () => {
  const {storage, lock} = fixture();
  const flush = vi.fn().mockResolvedValueOnce(undefined).mockRejectedValueOnce(Error("disk unavailable")).mockResolvedValue(undefined);
  registerStoragePersistence(storage, {backend: "IndexedDB", pending: false, error: undefined, resetting: false, flush,
    retry: async () => undefined, assertCurrent: async () => undefined, reset: async () => undefined, subscribe: () => () => undefined, close: () => undefined});
  await expect(retireLegacyConversationDeletionBatches(storage, lock)).rejects.toThrow("disk unavailable");
  expect(discoverConversationDeletionBatches(storage)).toEqual([batch]);
  expect(readConversationDeletion(storage, requests[0]!.conversationId)).toEqual(record(requests[0]!));
  await retireLegacyConversationDeletionBatches(storage, lock);
  expect(discoverConversationDeletionBatches(storage)).toEqual([]);
});

it("shares one migration between simultaneous views of the same storage", async () => {
  const {storage, lock} = fixture();
  await Promise.all([retireLegacyConversationDeletionBatches(storage, lock), retireLegacyConversationDeletionBatches(storage, lock)]);
  expect(discoverConversationDeletionBatches(storage)).toEqual([]);
});

it("prevents a previously opened owner from continuing a retired clear", async () => {
  const {storage, lock} = fixture();
  const deletions = {status: vi.fn(), delete: vi.fn()};
  const owner = new ConversationDeletionBatchController(binding, false, {deletions, cleanup: vi.fn(), lock}, storage);
  await retireLegacyConversationDeletionBatches(storage, lock);
  expect(await owner.continueOriginal()).toBe(false);
  expect(deletions.status).not.toHaveBeenCalled();
  expect(deletions.delete).not.toHaveBeenCalled();
  owner.dispose();
});
