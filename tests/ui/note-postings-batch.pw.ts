import {test,expect} from "@playwright/test";

test("posting batches combine additions and removals atomically without repeated gram I/O",async ({page}) => {
  await page.goto("/@vite/client");
  const result = await page.evaluate(async () => {
    const {catalogPostingUpdater} = await import("/src/core/features/notes/gateways/browserCatalogPostings.ts");
    const {catalogRequest,catalogTransactionDone,createCatalogStores} = await import("/src/core/features/notes/gateways/browserNoteCatalogStore.ts");
    const name = "posting-batch-" + crypto.randomUUID();
    const db = await new Promise<IDBDatabase>((resolve,reject) => {
      const request = indexedDB.open(name,1);request.onupgradeneeded = () => createCatalogStores(request.result);
      request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);
    });
    const metrics = {gets: 0,puts: 0,deletes: 0};
    const originalGet = IDBObjectStore.prototype.get, originalPut = IDBObjectStore.prototype.put, originalDelete = IDBObjectStore.prototype.delete;
    IDBObjectStore.prototype.get = function(...args) {if (this.transaction.db.name === name && this.name === "catalogPostings") metrics.gets++;return originalGet.apply(this,args);};
    IDBObjectStore.prototype.put = function(...args) {if (this.transaction.db.name === name && this.name === "catalogPostings") metrics.puts++;return originalPut.apply(this,args);};
    IDBObjectStore.prototype.delete = function(...args) {if (this.transaction.db.name === name && this.name === "catalogPostings") metrics.deletes++;return originalDelete.apply(this,args);};
    const members = new Set(["gram"]), empty = new Set<string>();
    const read = async () => {
      const tx = db.transaction(["catalogPostings","catalogCounts"],"readonly"), done = catalogTransactionDone(tx);
      const [posting,count] = await Promise.all([catalogRequest(tx.objectStore("catalogPostings").get("gram")),catalogRequest(tx.objectStore("catalogCounts").get("gram"))]);
      await done;return {posting: [...posting as Uint32Array ?? []],count: count ?? 0};
    };
    try {
      const tx = db.transaction(["catalogPostings","catalogCounts"],"readwrite"), done = catalogTransactionDone(tx), update = catalogPostingUpdater(tx);
      await update(2,empty,members);await update(4,empty,members);await update(2,members,empty);await update(6,empty,members);
      update.flush();await done;
      const combined = {...metrics}, saved = await read();
      const rollback = db.transaction(["catalogPostings","catalogCounts"],"readwrite"), rolledBack = catalogTransactionDone(rollback), remove = catalogPostingUpdater(rollback);
      await remove(4,members,empty);await remove(6,members,empty);remove.flush();rollback.abort();await rolledBack.catch(() => undefined);
      const retained = await read();
      const deletion = db.transaction(["catalogPostings","catalogCounts"],"readwrite"), deleted = catalogTransactionDone(deletion), drop = catalogPostingUpdater(deletion);
      await drop(4,members,empty);await drop(6,members,empty);drop.flush();await deleted;
      return {combined,saved,retained,deleted: await read()};
    } finally {
      IDBObjectStore.prototype.get = originalGet;IDBObjectStore.prototype.put = originalPut;IDBObjectStore.prototype.delete = originalDelete;
      db.close();await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});
    }
  });
  expect(result.combined).toEqual({gets: 1,puts: 1,deletes: 0});
  expect(result.saved).toEqual({posting: [4,6],count: 2});
  expect(result.retained).toEqual(result.saved);
  expect(result.deleted).toEqual({posting: [],count: 0});
});

test("net unchanged membership skips new writes and legacy conversion, real changes remain atomic",async ({page}) => {
  await page.goto("/@vite/client");
  const result = await page.evaluate(async () => {
    const {catalogPostingUpdater} = await import("/src/core/features/notes/gateways/browserCatalogPostings.ts");
    const {createIndexedPostingStore} = await import("/src/core/features/notes/gateways/browserCatalogPostingRows.ts");
    const {browserCatalogStores,catalogRequest,catalogTransactionDone,createCatalogStores} = await import("/src/core/features/notes/gateways/browserNoteCatalogStore.ts");
    const name = "posting-net-noop-" + crypto.randomUUID();
    const db = await new Promise<IDBDatabase>((resolve,reject) => {
      const request = indexedDB.open(name,1);request.onupgradeneeded = () => {createCatalogStores(request.result);createIndexedPostingStore(request.result);};
      request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);
    });
    const seed = db.transaction([...browserCatalogStores],"readwrite"), seeded = catalogTransactionDone(seed);
    seed.objectStore("catalogPostings").put(Uint32Array.of(2,4),"old");seed.objectStore("catalogCounts").put(2,"old");
    seed.objectStore("catalogPostingRows").put({schema: 1,gram: "new",count: 2,value: Uint32Array.of(2,4)});await seeded;
    const metrics = {gets: 0,puts: 0,deletes: 0}, originals: (() => void)[] = [];
    for (const [method,label] of [["get","gets"],["put","puts"],["delete","deletes"]] as const) {
      const original = IDBObjectStore.prototype[method];
      IDBObjectStore.prototype[method] = function(...args: unknown[]) {
        if (this.transaction.db.name === name) metrics[label]++;
        return Reflect.apply(original,this,args);
      };
      originals.push(() => {IDBObjectStore.prototype[method] = original;});
    }
    const members = new Set(["old","new"]), empty = new Set<string>();
    const read = async () => {
      const tx = db.transaction([...browserCatalogStores],"readonly"), done = catalogTransactionDone(tx);
      const [old,oldCount,migrated,current] = await Promise.all([
        catalogRequest(tx.objectStore("catalogPostings").get("old")),catalogRequest(tx.objectStore("catalogCounts").get("old")),
        catalogRequest(tx.objectStore("catalogPostingRows").get("old")),catalogRequest(tx.objectStore("catalogPostingRows").get("new")),
      ]);
      await done;return {old: [...old as Uint32Array ?? []],oldCount: oldCount ?? 0,migrated: migrated ? [...migrated.value] : [],current: [...current.value]};
    };
    try {
      const tx = db.transaction([...browserCatalogStores],"readwrite"), done = catalogTransactionDone(tx), update = catalogPostingUpdater(tx);
      await update(2,members,empty);await update(2,empty,members);await update(6,empty,members);await update(6,members,empty);update.flush();await done;
      const noops = {...metrics}, unchanged = await read();
      for (const key of ["gets","puts","deletes"] as const) metrics[key] = 0;
      const changed = db.transaction([...browserCatalogStores],"readwrite"), saved = catalogTransactionDone(changed), apply = catalogPostingUpdater(changed);
      await apply(2,members,empty);await apply(6,empty,members);apply.flush();await saved;
      const writes = {...metrics}, after = await read();
      const rollback = db.transaction([...browserCatalogStores],"readwrite"), rolledBack = catalogTransactionDone(rollback), remove = catalogPostingUpdater(rollback);
      await remove(4,members,empty);await remove(8,empty,members);remove.flush();rollback.abort();await rolledBack.catch(() => undefined);
      return {noops,unchanged,writes,after,retained: await read()};
    } finally {
      for (const restore of originals) restore();db.close();
      await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});
    }
  });
  expect(result.noops).toEqual({gets: 3,puts: 0,deletes: 0});
  expect(result.unchanged).toEqual({old: [2,4],oldCount: 2,migrated: [],current: [2,4]});
  expect(result.writes).toEqual({gets: 3,puts: 2,deletes: 2});
  expect(result.after).toEqual({old: [],oldCount: 0,migrated: [4,6],current: [4,6]});
  expect(result.retained).toEqual(result.after);
});
