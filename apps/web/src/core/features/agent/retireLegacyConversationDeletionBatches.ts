import { appStorageScopedKey } from "../../runtime/appStorage";
import { flushStorage, storagePersistence } from "../../runtime/storagePersistence";
import { announceConversationDeletionRecovery, readConversationDeletion, sameDeletionIntent } from "./conversationDeletionPending";
import { deletionBatchScope, discoverConversationDeletionBatches, readConversationDeletionBatch,
  type ConversationDeletionBatch } from "./conversationDeletionBatchPending";
import { assertConversationOperationStorageCurrent, browserConversationOperationLock,
  type ConversationOperationLock } from "./conversationOperationLock";

const batchKey = (id: string) => `research-assistant:conversation-deletion-batch:v1:${encodeURIComponent(id)}`;
const memberKey = (id: string) => `research-assistant:conversation-deletion:v1:${encodeURIComponent(id)}`;
const retiredKey = (id: string) => `research-assistant:conversation-deletion-batch-retired:v1:${encodeURIComponent(id)}`;
const defaultLock = browserConversationOperationLock("DELETION_BATCH");
const retiring = new WeakMap<Storage, Promise<void>>();

/** Revoke old resumable clear authorizations without discarding their evidence.
 * Existing batch and member owners use these same locks and verify their saved
 * records before performing another operation. A durable copy precedes removal. */
export async function retireLegacyConversationDeletionBatches(storage: Storage, lock = defaultLock): Promise<void> {
  const pending = retiring.get(storage);
  if (pending) return pending;
  const operation = (async () => {
    const persistence = storagePersistence(storage);
    if (persistence?.error) await persistence.retry();
    for (const batch of discoverConversationDeletionBatches(storage)) await retire(storage, lock, batch);
  })();
  retiring.set(storage, operation);
  try { await operation; } finally { if (retiring.get(storage) === operation) retiring.delete(storage); }
}

async function retire(storage: Storage, lock: ConversationOperationLock, batch: ConversationDeletionBatch): Promise<void> {
  const scope = appStorageScopedKey(storage, `research-assistant:conversation-deletion-batch-lock:v1:${deletionBatchScope(batch.binding, batch.internal)}`);
  const admission = appStorageScopedKey(storage, "research-assistant:conversation-deletion-batch-admission:v1");
  const members = [...batch.requests].sort((a, b) => a.conversationId.localeCompare(b.conversationId));
  const withMembers = async (index: number): Promise<void> => {
    if (index < members.length) {
      const name = appStorageScopedKey(storage, `research-assistant:conversation-deletion-lock:v1:${encodeURIComponent(members[index]!.conversationId)}`);
      await lock.run(name, () => withMembers(index + 1)); return;
    }
    const persistence = storagePersistence(storage);
    if (persistence?.error) await persistence.retry();
    await assertConversationOperationStorageCurrent(storage, "DELETION_BATCH");
    const current = readConversationDeletionBatch(storage, batch.batchId);
    if (!current) return;
    if (JSON.stringify(current) !== JSON.stringify(batch)) throw Error("DELETION_BATCH_CONFLICT");
    const records = members.flatMap(request => {
      const record = readConversationDeletion(storage, request.conversationId);
      return record && sameDeletionIntent(record.intent, request) ? [record] : [];
    });
    const previous = storage.getItem(retiredKey(batch.batchId));
    if (previous !== null) {
      let backup: {batch?: unknown; records?: unknown};
      try { backup = JSON.parse(previous) as typeof backup; } catch { throw Error("DELETION_BATCH_CONFLICT"); }
      if (!backup || JSON.stringify(backup.batch) !== JSON.stringify(current) || !Array.isArray(backup.records))
        throw Error("DELETION_BATCH_CONFLICT");
      const savedRecords: unknown[] = backup.records;
      if (records.some(record => !savedRecords.some(saved => JSON.stringify(saved) === JSON.stringify(record)))) throw Error("DELETION_BATCH_CONFLICT");
    } else storage.setItem(retiredKey(batch.batchId), JSON.stringify({batch: current, records}));
    await flushStorage(storage);
    for (const record of records) storage.removeItem(memberKey(record.intent.conversationId));
    storage.removeItem(batchKey(batch.batchId));
    try { await flushStorage(storage); }
    catch (cause: unknown) {
      // Keep the active index visible until durable revocation can be retried.
      for (const record of records) storage.setItem(memberKey(record.intent.conversationId), JSON.stringify(record));
      storage.setItem(batchKey(batch.batchId), JSON.stringify(current));
      throw cause;
    }
    announceConversationDeletionRecovery(storage);
  };
  await lock.run(scope, () => lock.run(admission, () => withMembers(0)));
}
