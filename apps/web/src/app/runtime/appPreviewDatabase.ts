import {createIndexedPostingStore} from "../../core/features/notes/gateways/browserCatalogPostingRows";
import { createWritingRecordStores, indexedWritingRecords } from "../../core/features/writing-storage/indexedWritingRecords";
import { allWritingRecordStores, registerWritingRecords } from "../../core/features/writing-storage/browserWritingRecords";
import {createWritingCheckpointStore, upgradeWritingCheckpoints} from "../../core/features/writing-storage/upgradeWritingCheckpoints";
import { upgradeWritingRecords } from "../../core/features/writing-storage/upgradeWritingRecords";
import {createWritingCatalogStores, upgradeWritingCatalog} from "../../core/features/writing-storage/upgradeWritingCatalog";
import {emptyWritingCatalogState, writingCatalogStores} from "../../core/features/writing-storage/writingCatalogState";

import {installPreviewDatabaseFactory, PreviewStorageError, previewStorageError, type PreviewDatabase} from "../../core/runtime/previewIndexedDatabase";

export async function openAppPreviewDatabase(name: string): Promise<PreviewDatabase> {
  if (typeof indexedDB === "undefined") throw new PreviewStorageError("preview_storage_unavailable", "浏览器不支持 IndexedDB，预览未启动；请使用支持的浏览器。");
  const db = await new Promise<IDBDatabase>((resolve, reject) => {
    let blocked = false;
    let upgradeError: Error | undefined;
    // v6 creates empty indexed posting rows without reading legacy projections,
    // files or recovery checkpoints; older writers must release their owner.
    const request = indexedDB.open(name, 6);
    request.onupgradeneeded = event => {
      if (event.oldVersion < 6) createIndexedPostingStore(request.result);
      if (event.oldVersion < 1) { request.result.createObjectStore("entries"); request.result.createObjectStore("metadata"); }
      if (event.oldVersion < 3) createWritingCatalogStores(request.result);
      const prepareCatalog = () => upgradeWritingCatalog(request.transaction!, reason => { upgradeError = reason; });
      if (event.oldVersion < 2) {
        createWritingRecordStores(request.result);
        upgradeWritingRecords(request.transaction!, reason => { upgradeError = reason; },prepareCatalog);
      } else if (event.oldVersion < 3) prepareCatalog();
      if (event.oldVersion < 4) {
        createWritingCheckpointStore(request.result);
        upgradeWritingCheckpoints(request.transaction!, reason => { upgradeError = reason; });
      }
    };
    request.onerror = () => reject(upgradeError ?? previewStorageError(request.error));
    request.onblocked = () => { blocked = true; reject(new PreviewStorageError("preview_storage_blocked", "数据库升级被其他标签页阻挡，请关闭旧标签页后重试。")); };
    request.onsuccess = () => { if (blocked) request.result.close(); else resolve(request.result); };
  });
  const writing = indexedWritingRecords(db);
  db.onversionchange = () => { writing.stop(); db.close(); };
  return {
    managed: writing,
    bindStorage: storage => registerWritingRecords(storage,writing),
    close: () => { writing.stop(); db.close(); },
    readRevision: () => new Promise((resolve, reject) => {
      const tx = db.transaction("metadata", "readonly");
      let revision: number;
      const request = tx.objectStore("metadata").get("revision");
      request.onsuccess = () => { revision = request.result; };
      tx.oncomplete = () => {
        if (!Number.isSafeInteger(revision) || revision < 0) reject(new PreviewStorageError("preview_storage_corrupt", "预览存储修订号损坏，原记录已保留。"));
        else resolve(revision);
      };
      tx.onabort = tx.onerror = () => reject(previewStorageError(tx.error));
    }),
    load: () => new Promise((resolve, reject) => {
      const tx = db.transaction(["entries", "metadata"], "readwrite", {durability: "strict"});
      const store = tx.objectStore("entries"), meta = tx.objectStore("metadata");
      const entries = new Map<string, string>();
      let revision = 0, error: unknown;
      const current = meta.get("revision");
      current.onsuccess = () => {
        try {
          if (current.result === undefined) {
            // Fresh spaces start empty; completed previews have no legacy read path.
            meta.put(0, "revision");
          } else {
            if (!Number.isSafeInteger(current.result) || current.result < 0) throw new PreviewStorageError("preview_storage_corrupt", "预览存储修订号损坏，原记录已保留。");
            revision = current.result as number;
          }
          const cursor = store.openCursor();
          cursor.onsuccess = () => {
            const row = cursor.result;
            if (row) {
              if (typeof row.key !== "string" || typeof row.value !== "string") {
                error = new PreviewStorageError("preview_storage_corrupt", "预览存储记录格式损坏，原记录已保留。"); tx.abort(); return;
              }
              entries.set(row.key, row.value); row.continue();
            }
          };
        } catch (reason) { error = reason; tx.abort(); }
      };
      tx.oncomplete = () => resolve({revision, entries});
      tx.onabort = tx.onerror = () => reject(previewStorageError(error ?? tx.error));
    }),
    commit: (revision, changes, clear) => new Promise((resolve, reject) => {
      const tx = db.transaction(["entries", "metadata", ...(clear ? [...allWritingRecordStores,...writingCatalogStores] : [])], "readwrite", {durability: "strict"});
      const store = tx.objectStore("entries"), meta = tx.objectStore("metadata");
      let error: Error | undefined;
      const current = meta.get("revision");
      current.onsuccess = () => {
        if (current.result !== revision) {
          error = new PreviewStorageError("preview_storage_conflict", "其他标签页已更新此预览空间。当前修改仍保留在页面中，请先复制需要保留的内容，再刷新核对；未覆盖其他标签页的数据。");
          tx.abort(); return;
        }
        if (clear) {
          store.clear();
          for (const name of allWritingRecordStores) tx.objectStore(name).clear();
          for (const name of writingCatalogStores) tx.objectStore(name).clear();
          tx.objectStore("catalogState").put(emptyWritingCatalogState(),"current");
          const previous = meta.get("writingRevision");
          previous.onsuccess = () => meta.put((previous.result ?? 0) + 1, "writingRevision");
          const checkpointRevision = meta.get("writingCheckpointRevision");
          checkpointRevision.onsuccess = () => meta.put((checkpointRevision.result ?? 0) + 1, "writingCheckpointRevision");
        }
        for (const [key, value] of changes) { if (value === null) store.delete(key); else store.put(value, key); }
        meta.put(revision + 1, "revision");
      };
      // A successful request is not a saved receipt: wait for the entire transaction.
      tx.oncomplete = () => resolve(revision + 1);
      tx.onabort = tx.onerror = () => reject(error ?? previewStorageError(tx.error));
    }),
  };
}

installPreviewDatabaseFactory(openAppPreviewDatabase);
