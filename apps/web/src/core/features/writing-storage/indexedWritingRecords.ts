import { writingRecordStores, writingRecordRevision, type BrowserWritingRecords, type WritingRecordSnapshot } from "./browserWritingRecords";
import {createWritingCatalogOwner} from "./writingCatalogOwner";
import {applyWritingCatalogChanges, writingCatalogRuntimeForChanges} from "./writingCatalogWrites";
import {writingCatalogStores} from "./writingCatalogState";

function storageError(reason: unknown): Error {
  return Object.assign(new Error(`writing_storage_failed：写作记录未完成，已保存数据未改变。${reason instanceof Error ? `${reason.name}: ${reason.message}` : String(reason)}`, {cause: reason}), {code: "writing_storage_failed"});
}

/** Uses the owning preview connection so reset also clears file records. */
export function indexedWritingRecords(database: IDBDatabase): BrowserWritingRecords {
  const catalog = createWritingCatalogOwner(database);
  let stopped = false;
  const pending = new Set<Promise<unknown>>();
  let writes = 0;
  const listeners = new Set<() => void>();
  const publish = () => { for (const listener of listeners) listener(); };
  function track<T>(operation: () => Promise<T>, write = false): Promise<T> {
    if (stopped) return Promise.reject(Object.assign(new Error("writing_storage_closed：写作运行环境已关闭。"), {code: "writing_storage_closed"}));
    let result: Promise<T>;
    try {
      result = operation().catch(reason => {
        if (reason instanceof Error && "code" in reason && typeof reason.code === "string") throw reason;
        throw storageError(reason);
      });
    } catch (reason) { return Promise.reject(storageError(reason)); }
    pending.add(result); if (write) { writes++; publish(); }
    // Observe both outcomes without retaining an unhandled finally promise.
    const completed = () => { pending.delete(result); if (write) { writes--; publish(); } };
    void result.then(completed, completed);
    return result;
  }
  return {
    noteCatalog: {
      queryWritingNoteCatalog: query => track(() => catalog.queryWritingNoteCatalog(query)),
      prepareWritingNoteCatalog: options => track(() => catalog.prepareWritingNoteCatalog(options)),
    },
    get pending() { return writes > 0; },
    subscribe(changed) { listeners.add(changed); return () => { listeners.delete(changed); }; },
    stop() { stopped = true; },
    async flush() { while (pending.size) await Promise.all([...pending]); },
    read: selections => track(() => new Promise((resolve, reject) => {
      const revisionKey = writingRecordRevision(selections.map(selection => selection.store));
      const names = [...new Set(["metadata", ...selections.map(selection => selection.store)])];
      const tx = database.transaction(names, "readonly");
      const rows: WritingRecordSnapshot["rows"] = {};
      let revision = 0, cause: Error | undefined;
      const version = tx.objectStore("metadata").get(revisionKey);
      version.onsuccess = () => {
        revision = version.result ?? 0;
        if (!Number.isSafeInteger(revision) || revision < 0) { cause = storageError(new Error("writing_revision_invalid")); tx.abort(); }
      };
      for (const selection of selections) {
        if (rows[selection.store]) { cause = storageError(new Error("duplicate store selection")); tx.abort(); break; }
        const found: unknown[] = []; rows[selection.store] = found;
        const store = tx.objectStore(selection.store);
        if (selection.keys) {
          for (const key of selection.keys) {
            const request = store.get(key);
            request.onsuccess = () => { if (request.result !== undefined) found.push(request.result); };
          }
        } else {
          const cursor = selection.manuscriptId === undefined ? store.openCursor()
            : store.index("manuscriptId").openCursor(IDBKeyRange.only(selection.manuscriptId));
          cursor.onsuccess = () => { const row = cursor.result; if (row) { found.push(row.value); row.continue(); } };
        }
      }
      tx.oncomplete = () => resolve({revision, rows});
      tx.onabort = tx.onerror = () => reject(cause ?? storageError(tx.error));
    })),
    commit: (expectedRevision, changes) => track(async () => {
      const revisionKey = writingRecordRevision(changes.map(change => change.store));
      const runtime = await writingCatalogRuntimeForChanges(database,changes);
      return new Promise<void>((resolve, reject) => {
      const tx = database.transaction([...new Set(["metadata", ...changes.map(change => change.store),...(runtime ? ["writingProjects","writingFiles",...writingCatalogStores] : [])])], "readwrite", {durability: "strict"});
      const meta = tx.objectStore("metadata"), current = meta.get(revisionKey);
      let cause: Error | undefined;
      current.onsuccess = () => {
        if ((current.result ?? 0) !== expectedRevision) {
          cause = Object.assign(new Error("writing_revision_conflict：其他操作已更新写作数据，请保留输入并重新读取。"), {code: "writing_revision_conflict"});
          tx.abort(); return;
        }
        void (async () => {
          if (runtime) await applyWritingCatalogChanges(tx,changes,runtime);
          for (const change of changes) {
            const store = tx.objectStore(change.store);
            if (change.value === undefined) store.delete(change.key); else store.put(change.value, change.key);
          }
          meta.put(expectedRevision + 1, revisionKey);
        })().catch(reason => { cause = storageError(reason); try { tx.abort(); } catch { /* already aborted */ } });
      };
      tx.oncomplete = () => resolve();
      tx.onabort = tx.onerror = () => reject(cause ?? storageError(tx.error));
      });
    }, true),
  };
}

/** Versioned schema setup; file metadata never contains content. */
export function createWritingRecordStores(database: IDBDatabase): void {
  for (const name of writingRecordStores) {
    const store = database.createObjectStore(name);
    if (name === "writingFiles" || name === "writingDrafts") store.createIndex("manuscriptId", "manuscriptId");
  }
}
