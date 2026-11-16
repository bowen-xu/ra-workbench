import {publicPackagePath} from "./projectPaths";
import {test,expect} from "@playwright/test";
import {resolve} from "node:path";
test("indexed counts avoid duplicate writes and payload reads, reject missing count metadata",async ({page}) => {
  await page.goto("/@vite/client");
  const result = await page.evaluate(async coreFile => {
    const core = await import("/@fs/"+coreFile), gateway = "/src/core/features/notes/gateways/";
    const {openIndexedDatabase} = await import(gateway+"browserNotesDatabase.ts");
    const {indexedPostingStore} = await import(gateway+"browserCatalogPostingRows.ts");
    const {catalogRequest,catalogTransactionDone} = await import(gateway+"browserCatalogRequests.ts");
    const name = "note-posting-rows-"+crypto.randomUUID(),db = await openIndexedDatabase(name);
    const writes: Record<string,number> = {}, reads: Record<string,number> = {};
    const get = IDBObjectStore.prototype.get, put = IDBObjectStore.prototype.put;
    IDBObjectStore.prototype.get = function(...args) {if (this.transaction.db.name === name) reads[this.name] = (reads[this.name]??0)+1;return get.apply(this,args);};
    IDBObjectStore.prototype.put = function(...args) {if (this.transaction.db.name === name) writes[this.name] = (writes[this.name]??0)+1;return put.apply(this,args);};
    let raw: IDBDatabase | undefined;
    const assert = (value: unknown,message: string) => {if (!value) throw Error(message);};
    try {
      const id = core.notePageId("page"),timestamp = "2026-10-24T00:00:00Z";
      await db.commit(0,[{store: "pages",key: id,value: {id,title: "Title",position: 0,status: "active",isFavorite: false,revision: 0,schemaVersion: 1,projectIds: [],tagIds: [],createdAt: timestamp,updatedAt: timestamp}},
        {store: "documents",key: id,value: core.createNoteDocumentFromText(id,"sentinel",() => core.noteBlockId("block"))}]);
      assert((await db.queryCatalog({category: "all",view: "recent",search: "sentinel",limit: 20})).items.length === 1,"initial indexed search");
      assert(writes[indexedPostingStore]>0 && !writes.catalogCounts && !writes.catalogPostings,"new rows duplicated legacy writes");
      const initialWrites = {...writes};for (const key of Object.keys(reads)) delete reads[key];
      await db.queryCatalog({category: "all",view: "recent",search: "sentinel",limit: 20});
      assert(reads[indexedPostingStore] === 1 && !reads.catalogCounts && !reads.catalogPostings && !reads.documents,"count selection read posting payloads or canonical bodies");
      const hot = {...reads};
      raw = await new Promise<IDBDatabase>((resolve,reject) => {const request = indexedDB.open(name);request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);});
      const tx = raw.transaction(indexedPostingStore,"readwrite"), done = catalogTransactionDone(tx), store = tx.objectStore(indexedPostingStore);
      const row = await catalogRequest(store.get("sen"));const {count: _count,...bad} = row;
      store.put(bad);await done;
      let diagnosed = false;
      try {await db.queryCatalog({category: "all",view: "recent",search: "sentinel",limit: 20});}
      catch (error) {diagnosed = error instanceof Error && error.message.includes("note_catalog_posting_row_invalid");}
      assert(diagnosed,"missing count index entry became a false empty result");
      const restore = raw.transaction(indexedPostingStore,"readwrite"), restored = catalogTransactionDone(restore);restore.objectStore(indexedPostingStore).put(row);await restored;
      assert((await db.queryCatalog({category: "all",view: "recent",search: "sentinel",limit: 20})).items.length === 1,"repaired row did not remain readable");
      return {initialWrites,hot,diagnosed};
    } finally {
      IDBObjectStore.prototype.get = get;IDBObjectStore.prototype.put = put;raw?.close();db.close();
      await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});
    }
  },publicPackagePath("@research-assistant/core/notes"));
  expect(result.diagnosed).toBe(true);expect(result.hot.catalogPostingRows).toBe(1);
  console.log("indexed-posting-rows",JSON.stringify(result));
});
