import type {Page} from "@playwright/test";

/** Real IndexedDB upgrade and production gateway. No alternative recovery model. */
export async function seedCheckpointV3(page: Page, options: {prefix: string; count: number; invalid?: boolean; version?: 1 | 2 | 3}) {
  await page.goto("/@vite/client");
  return page.evaluate(async options => {
    const {createWritingRecordStores} = await import("/ra-workbench/apps/web/src/core/features/writing-storage/indexedWritingRecords.ts");
    const {createWritingCatalogStores} = await import("/ra-workbench/apps/web/src/core/features/writing-storage/upgradeWritingCatalog.ts");
    const {emptyWritingCatalogState} = await import("/ra-workbench/apps/web/src/core/features/writing-storage/writingCatalogState.ts");
    const {encodeMockState} = await import("/ra-workbench/apps/web/src/core/runtime/mockStateCodec.ts");
    const name = "research-assistant.preview." + encodeURIComponent(options.prefix);
    const rows = new Map();
    for (let i = 0; i < options.count; i++) {
      const checkpoint = {schemaVersion: 1, manuscriptId: "checkpoint-project-" + Math.floor(i / 2),
        relativePath: `source-${i}.tex`, version: 3, content: "candidate 😀\r\n" + "c".repeat(2048),
        reviewBaseline: "baseline\r\n" + "b".repeat(2048), savedSha256: "a".repeat(64), savedVersion: 1,
        operations: [{operationId: "operation-a", conversationId: "conversation-a", requestId: "request-a", resultSha256: "b".repeat(64)}],
        saveIntent: {operationId: "save-a", version: 2, expectedSha256: "a".repeat(64), targetSha256: "b".repeat(64)}};
      rows.set(JSON.stringify([checkpoint.manuscriptId, checkpoint.relativePath]), {revision: 7, checkpoint});
    }
    if (options.invalid) rows.set("wrong-identity", [...rows.values()][0]);
    const original = encodeMockState(rows);
    Object.assign(window, {__checkpointLegacyBundle: original});
    const db = await new Promise<IDBDatabase>((resolve, reject) => {
      const request = indexedDB.open(name, options.version ?? 3);
      request.onupgradeneeded = () => {
        request.result.createObjectStore("entries"); request.result.createObjectStore("metadata");
        if ((options.version ?? 3) >= 2) createWritingRecordStores(request.result);
        if ((options.version ?? 3) >= 3) createWritingCatalogStores(request.result);
      };
      request.onsuccess = () => resolve(request.result); request.onerror = () => reject(request.error);
    });
    await new Promise<void>((resolve, reject) => {
      const tx = db.transaction(["entries", "metadata", ...((options.version ?? 3) >= 3 ? ["catalogState"] : [])], "readwrite");
      tx.objectStore("entries").put(original, "writing-checkpoints");
      tx.objectStore("entries").put("keep-preference", "preference");
      tx.objectStore("metadata").put(0, "revision"); tx.objectStore("metadata").put(12, "writingRevision");
      if ((options.version ?? 3) >= 3) tx.objectStore("catalogState").put(emptyWritingCatalogState(), "current");
      tx.oncomplete = () => resolve(); tx.onabort = () => reject(tx.error);
    });
    db.close();
    return {name, count: rows.size, bytes: original.length};
  }, options);
}

export async function verifyCheckpointUpgrade(page: Page, prefix: string, failCopy = false) {
  return page.evaluate(async ({prefix, failCopy}) => {
    const {createIndexedPreviewStorage} = await import("/ra-workbench/apps/web/src/core/runtime/previewIndexedStorage.ts");
    const {storagePersistence} = await import("/ra-workbench/apps/web/src/core/runtime/storagePersistence.ts");
    const {writingRecords} = await import("/ra-workbench/apps/web/src/core/public/writingStorage.ts");
    const {createBrowserWritingCheckpointGateway} = await import("/plugins/ra-writing-projects/src/features/writing/gateway/browserWritingCheckpointGateway.ts");
    const name = "research-assistant.preview." + encodeURIComponent(prefix);
    const add = IDBObjectStore.prototype.add;
    let copied = 0, errorCode = "";
    if (failCopy) IDBObjectStore.prototype.add = function(...args) {
      if (this.name === "writingCheckpoints" && ++copied === 2) throw new DOMException("synthetic quota", "QuotaExceededError");
      return add.apply(this, args);
    };
    let storage: Storage | undefined;
    try { storage = await createIndexedPreviewStorage(prefix); }
    catch (reason) { errorCode = reason.code; }
    finally { IDBObjectStore.prototype.add = add; }
    if (!storage) {
      const db = await new Promise<IDBDatabase>((resolve, reject) => { const r = indexedDB.open(name); r.onsuccess = () => resolve(r.result); r.onerror = () => reject(r.error); });
      const result = await new Promise<{legacy: boolean; version: number; hasStore: boolean}>((resolve, reject) => {
        const tx = db.transaction("entries"), req = tx.objectStore("entries").get("writing-checkpoints");
        tx.oncomplete = () => resolve({legacy: typeof req.result === "string" && req.result === window.__checkpointLegacyBundle, version: db.version, hasStore: db.objectStoreNames.contains("writingCheckpoints")}); tx.onabort = () => reject(tx.error);
      });
      db.close(); return {errorCode, ...result};
    }
    try {
      const gateway = createBrowserWritingCheckpointGateway(storage);
      const before = await gateway.list("checkpoint-project-5");
      if (before.length !== 2 || before.some(row => row.revision !== 7 || row.checkpoint.content === row.checkpoint.reviewBaseline || row.checkpoint.saveIntent?.operationId !== "save-a")) throw Error("recovery state lost");
      const snapshot = await writingRecords(storage).read([]);
      if (snapshot.revision !== 12) throw Error("checkpoint migration changed authoring revision");
      return {errorCode, legacy: storage.getItem("writing-checkpoints") !== null, preference: storage.getItem("preference"), revision: before[0].revision, count: before.length};
    } finally { storagePersistence(storage).close(); await storagePersistence(storage).flush(); }
  }, {prefix, failCopy});
}

export async function installCheckpointMetrics(page: Page) {
  await page.addInitScript(() => {
    const metrics = {requests: {} as Record<string, number>, rows: 0, puts: 0};
    Object.assign(window, {__checkpointMetrics: metrics});
    const track = (target: IDBObjectStore | IDBIndex, method: string, request: IDBRequest) => {
      if ((target instanceof IDBIndex ? target.objectStore.name : target.name) !== "writingCheckpoints") return request;
      const key = `${target instanceof IDBIndex ? "index" : "store"}.${method}`;
      metrics.requests[key] = (metrics.requests[key] ?? 0) + 1;
      request.addEventListener("success", () => { if (request.result !== undefined && request.result !== null) metrics.rows++; });
      return request;
    };
    const get = IDBObjectStore.prototype.get, cursor = IDBObjectStore.prototype.openCursor, indexCursor = IDBIndex.prototype.openCursor, put = IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.get = function(...args) { return track(this, "get", get.apply(this, args)); };
    IDBObjectStore.prototype.openCursor = function(...args) { return track(this, "openCursor", cursor.apply(this, args)); };
    IDBIndex.prototype.openCursor = function(...args) { return track(this, "openCursor", indexCursor.apply(this, args)); };
    IDBObjectStore.prototype.put = function(...args) { if (this.name === "writingCheckpoints") metrics.puts++; return put.apply(this, args); };
  });
}
