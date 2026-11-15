import { storagePersistence } from "../../runtime/storagePersistence";

export interface ConversationOperationLock {
  run<T>(name: string, operation: () => Promise<T>): Promise<T>;
}
/** One standard external lock adapter shared by catalog recovery owners. */
export function browserConversationOperationLock(errorPrefix: string, options: { readonly waitMs?: number } = {}): ConversationOperationLock {
  return { async run(name, operation) {
    if (typeof navigator === "undefined" || !navigator.locks) throw new Error(`${errorPrefix}_LOCK_UNAVAILABLE`);
    if (options.waitMs !== undefined) {
      const abort = new AbortController();
      const timeout = setTimeout(() => abort.abort(), options.waitMs);
      try {
        return await navigator.locks.request(name, {mode: "exclusive", signal: abort.signal}, async lock => {
          clearTimeout(timeout);
          if (!lock) throw new Error(`${errorPrefix}_LOCK_BUSY`);
          return operation();
        });
      } catch (cause) {
        if (abort.signal.aborted) throw new Error(`${errorPrefix}_LOCK_BUSY`);
        throw cause;
      } finally { clearTimeout(timeout); }
    }
    return navigator.locks.request(name, {mode: "exclusive", ifAvailable: true}, async lock => {
      if (!lock) throw new Error(`${errorPrefix}_LOCK_BUSY`);
      return operation();
    });
  } };
}
export async function assertConversationOperationStorageCurrent(storage: Storage, errorPrefix: string): Promise<void> {
  const persistence = storagePersistence(storage);
  if (!persistence) return; // localStorage reads are live across the origin.
  if (!persistence.assertCurrent) throw new Error(`${errorPrefix}_STORAGE_CHECK_UNAVAILABLE`);
  try { await persistence.assertCurrent(); }
  catch (cause: unknown) {
    // The existing preview port predates the shared catalog adapter.
    if (cause instanceof Error && cause.message === "FORK_RECOVERY_STORAGE_STALE") throw new Error(`${errorPrefix}_STORAGE_STALE`);
    throw cause;
  }
}
