import { PREVIEW_STORAGE_PREFIX, registerAppStoragePrefix } from "./appStorage";
import { openPreviewDatabase, previewStorageError } from "./previewIndexedDatabase";
import { registerStoragePersistence, type StoragePersistence } from "./storagePersistence";

/** Synchronous reads are a hydrated projection; flush is the durable receipt. */
export async function createIndexedPreviewStorage(prefix = PREVIEW_STORAGE_PREFIX): Promise<Storage> {
  const db = await openPreviewDatabase(`research-assistant.preview.${encodeURIComponent(prefix)}`);
  let loaded;
  try {
    loaded = await db.load();
  } catch (reason) { db.close(); throw reason; }
  const values = new Map(loaded.entries);
  let revision = loaded.revision, pending = new Map<string, string | null>(), clear = false;
  let operation: Promise<void> | undefined, failure: Error | undefined, closing = false, closed = false, resetting = false;
  const listeners = new Set<() => void>();
  const publish = () => { for (const changed of listeners) changed(); };
  const unsubscribeWriting = db.managed?.subscribe(publish);
  function check(): void {
    if (closed || closing) throw new Error("preview_storage_closed：预览运行环境已关闭。");
    if (failure) throw failure;
  }
  function schedule(): void {
    publish();
    // Batch this task's synchronous writes into one transaction, including compound mock saves.
    queueMicrotask(() => { if (!failure && !operation) void drain().catch(() => { /* Exposed by status and flush, never reported as saved. */ }); });
  }
  async function drain(): Promise<void> {
    if (failure) throw failure;
    if (operation) return operation;
    if (!clear && pending.size === 0) return;
    const write = async () => {
      while (clear || pending.size) {
        const batch = pending, reset = clear;
        pending = new Map(); clear = false;
        try { revision = await db.commit(revision, batch, reset); }
        catch (reason) {
          // Keep the failed batch ahead of newer writes; a queued clear supersedes older values.
          if (!clear) pending = new Map([...batch, ...pending]);
          clear ||= reset;
          failure = previewStorageError(reason); throw failure;
        }
      }
    };
    operation = write(); publish();
    try { await operation; }
    finally { operation = undefined; publish(); }
  }
  const storage: Storage = {
    get length() { return values.size; },
    key: index => [...values.keys()][index] ?? null,
    getItem: key => values.get(String(key)) ?? null,
    setItem(key, value) { check(); key = String(key); value = String(value); if (values.get(key) === value) return; values.set(key, value); pending.set(key, value); schedule(); },
    removeItem(key) { check(); key = String(key); if (!values.has(key)) return; values.delete(key); pending.set(key, null); schedule(); },
    clear() { check(); values.clear(); pending.clear(); clear = true; schedule(); },
  };
  async function finishReset(): Promise<void> {
    await flush(); closing = true;
    db.managed?.stop(); await db.managed?.flush();
    values.clear(); pending.clear(); clear = true;
    await drain();
  }
  async function flush(): Promise<void> { await db.managed?.flush(); await drain(); }
  const port: StoragePersistence = {
    backend: "IndexedDB",
    get pending() { return !!operation || clear || pending.size > 0 || !!db.managed?.pending; }, get error() { return failure; },
    get resetting() { return resetting; },
    flush,
    async assertCurrent() {
      check(); await flush(); check();
      if (await db.readRevision() !== revision) throw new Error("FORK_RECOVERY_STORAGE_STALE");
    },
    async retry() { failure = undefined; publish(); if (resetting) await finishReset(); else await drain(); },
    async reset(beforeClear) {
      await flush(); resetting = true;
      try { beforeClear?.(); }
      catch (reason) { failure = previewStorageError(reason); publish(); throw failure; }
      await finishReset();
    },
    subscribe(changed) { listeners.add(changed); return () => { listeners.delete(changed); }; },
    close() {
      closing = true;
      db.managed?.stop();
      void flush().then(() => { closed = true; unsubscribeWriting?.(); db.close(); }, () => { /* Keep the failed draft and connection available for recovery. */ });
    },
  };
  registerStoragePersistence(storage, port);
  db.bindStorage?.(storage);
  registerAppStoragePrefix(storage, prefix);
  return storage;
}
