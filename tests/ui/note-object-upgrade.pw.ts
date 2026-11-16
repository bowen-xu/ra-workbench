import {publicPackagePath} from "./projectPaths";
import {test,expect} from "@playwright/test";
import {resolve} from "node:path";

test("v3 database upgrades metadata only and excludes older writers",async ({page}) => {
  await page.goto("/@vite/client");
  const result = await page.evaluate(async files => {
    const core = await import("/@fs/" + files.core), data = await import("/@fs/" + files.data);
    const path = "/src/core/features/notes/gateways/";
    const {createCatalogStores,catalogTransactionDone,catalogRequest} = await import(path + "browserNoteCatalogStore.ts");
    const {loadBrowserNoteCatalogRuntime} = await import(path + "browserNoteCatalogRuntime.ts");
    const {openIndexedDatabase,noteStores} = await import(path + "browserNotesDatabase.ts");
    const runtime = await loadBrowserNoteCatalogRuntime(), name = "note-object-v3-" + crypto.randomUUID();
    const old = await new Promise<IDBDatabase>((resolve,reject) => {
      const request = indexedDB.open(name,3);
      request.onupgradeneeded = () => {
        for (const store of [...noteStores,"version"]) request.result.createObjectStore(store);
        request.transaction!.objectStore("sources").createIndex("pageId","pageId");
        request.transaction!.objectStore("links").createIndex("sourcePageId","sourcePageId");
        createCatalogStores(request.result);request.transaction!.objectStore("catalogHeaders").createIndex("reading",["item.target.kind","item.id"]);
      };
      request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);
    });
    let oldClosed = false;
    old.onversionchange = () => {oldClosed = true;old.close();};
    const seed = old.transaction(["pages","documents","catalogHeaders","catalogState","version"],"readwrite"), seeded = catalogTransactionDone(seed);
    for (let i = 0; i < 3; i++) {
      const id = core.notePageId("old-" + i), timestamp = "2026-10-24T00:00:00Z";
      const page = {id,title: "Old " + i,status: "active",isFavorite: false,position: i,revision: 17,schemaVersion: 1,projectIds: [],tagIds: [],createdAt: timestamp,updatedAt: timestamp};
      seed.objectStore("pages").put(page,id);
      seed.objectStore("documents").put(core.createNoteDocumentFromText(id,"Original body " + i,() => core.noteBlockId("old-b-" + i)),id);
      const header = data.projectNoteCatalogHeader({page,links: [],bodyFirstLine: "",ordinal: i + 1},runtime);
      seed.objectStore("catalogHeaders").put({...header,orderKeys: header.orderKeys.filter((key: readonly [string,unknown,string]) => !key[0].startsWith("objects/"))});
    }
    seed.objectStore("version").put(7,"current");
    seed.objectStore("catalogState").put({generation: 11,nextOrdinal: 4,collationVersion: runtime.collationVersion,normalizationVersion: runtime.normalizationVersion,
      gramVersion: data.NOTE_SEARCH_GRAM_VERSION,headersReady: true,searchReady: false,readingTitlesReady: true},"current");
    await seeded;
    const reads: Record<string,number> = {}, writes: Record<string,number> = {};
    const originalGet = IDBObjectStore.prototype.get, originalPut = IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.get = function(...args) {if (this.transaction.db.name === name) reads[this.name] = (reads[this.name] ?? 0) + 1;return originalGet.apply(this,args);};
    IDBObjectStore.prototype.put = function(...args) {if (this.transaction.db.name === name) writes[this.name] = (writes[this.name] ?? 0) + 1;return originalPut.apply(this,args);};
    let db;
    try {
      db = await openIndexedDatabase(name);
      const upgraded = {reads: {...reads},writes: {...writes}};
      for (const group of [reads,writes]) for (const key of Object.keys(group)) delete group[key];
      const list = await db.queryCatalog({category: "all",view: "recent",search: "",limit: 20});
      const listing = {reads: {...reads},writes: {...writes}};
      const objects = await db.searchNotes({limit: 1}), querying = {reads: {...reads},writes: {...writes}};
      const canonical = await db.read({pages: ["old-0"],documents: ["old-0"]});
      const oldVersionRejected = await new Promise<boolean>((resolve,reject) => {
        const request = indexedDB.open(name,3);request.onerror = () => resolve(request.error?.name === "VersionError");request.onsuccess = () => {request.result.close();reject(Error("old writer reopened"));};
      });
      return {oldClosed,oldVersionRejected,upgraded,listing,querying,listLength: list.items.length,objects: objects.items.map((item: {objectId: string}) => item.objectId),
        version: canonical.version,page: canonical.rows.pages["old-0"],body: core.noteDocumentPlainText(canonical.rows.documents["old-0"])};
    } finally {
      IDBObjectStore.prototype.get = originalGet;IDBObjectStore.prototype.put = originalPut;
      db?.close();old.close();await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});
    }
  },{core: publicPackagePath("@research-assistant/core/notes"),data: publicPackagePath("@research-assistant/data/notes")});
  expect(result.oldClosed).toBe(true);expect(result.oldVersionRejected).toBe(true);
  expect(result.upgraded.reads.documents ?? 0).toBe(0);expect(result.upgraded.reads.pages ?? 0).toBe(0);
  expect(result.upgraded.writes.documents ?? 0).toBe(0);expect(result.upgraded.writes.version ?? 0).toBe(0);
  expect(result.listing.writes.catalogHeaders ?? 0).toBe(0);expect(result.listing.reads.documents ?? 0).toBe(0);
  expect(result.querying.reads.documents ?? 0).toBe(0);expect(result.querying.reads.catalogBodies ?? 0).toBe(0);
  expect(result.listLength).toBe(3);expect(result.objects).toEqual(["old-0"]);expect(result.version).toBe(7);
  expect(result.page.revision).toBe(17);expect(result.body).toBe("Original body 0");
});
