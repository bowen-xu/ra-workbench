import {indexedPostingStore} from "../../apps/web/src/core/features/notes/gateways/browserCatalogPostingRows";
import {createNoteDocumentFromText,notePageId,noteBlockId,type NotePage} from "@research-assistant/core/notes";
import {applyBrowserNoteCatalogChanges} from "../../apps/web/src/core/features/notes/gateways/browserNoteCatalogWrites";
import {openIndexedDatabase,noteStores,type BrowserNotesDatabase,type NoteWrite} from "../../apps/web/src/core/features/notes/gateways/browserNotesDatabase";
import {loadBrowserNoteCatalogRuntime} from "../../apps/web/src/core/features/notes/gateways/browserNoteCatalogRuntime";
import {createCatalogStores,catalogRequest,catalogTransactionDone,catalogStores,type BrowserCatalogState} from "../../apps/web/src/core/features/notes/gateways/browserNoteCatalogStore";

function check(value: unknown,message: string): asserts value {if (!value) throw Error(message);}
function page(id: string): NotePage {return {id: notePageId(id),title: id,position: 0,status: "active",isFavorite: false,schemaVersion: 1,revision: 17,projectIds: [],tagIds: [],createdAt: "2026-10-24T00:00:00Z",updatedAt: "2026-10-24T00:00:00Z"};}
function changes(id: string,text: string): NoteWrite[] {const item = page(id);return [{store: "pages",key: id,value: item},{store: "documents",key: id,value: createNoteDocumentFromText(item.id,text,() => noteBlockId(id + "-block"))}];}
async function raw(name: string): Promise<IDBDatabase> {return new Promise((resolve,reject) => {const request = indexedDB.open(name);request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);});}

export async function verifyMixedPostingCodec() {
  const name = "note-codec-v4-" + crypto.randomUUID(), runtime = await loadBrowserNoteCatalogRuntime();
  const old = await new Promise<IDBDatabase>((resolve,reject) => {
    const request = indexedDB.open(name,4);request.onupgradeneeded = () => {
      for (const store of [...noteStores,"version"]) request.result.createObjectStore(store);
      request.transaction!.objectStore("sources").createIndex("pageId","pageId");request.transaction!.objectStore("links").createIndex("sourcePageId","sourcePageId");
      createCatalogStores(request.result);request.transaction!.objectStore("catalogHeaders").createIndex("reading",["item.target.kind","item.id"]);
    };request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);
  });
  const seed = [...changes("legacy-0","common legacy"),...changes("legacy-1","rare sentinel")];
  const tx = old.transaction([...noteStores,...catalogStores,"version"],"readwrite"), seeded = catalogTransactionDone(tx);
  await applyBrowserNoteCatalogChanges(tx,seed,runtime);
  for (const change of seed) tx.objectStore(change.store).put(change.value,change.key);
  const state = await catalogRequest(tx.objectStore("catalogState").get("current")) as BrowserCatalogState;
  tx.objectStore("catalogState").put({...state,headersReady: true,searchReady: true,readingTitlesReady: true,searchMetadataReady: true},"current");tx.objectStore("version").put(11,"current");
  await seeded;
  let oldClosed = false;old.onversionchange = () => {oldClosed = true;old.close();};
  let db: BrowserNotesDatabase | undefined, canonicalReads = 0, canonicalWrites = 0, projectionWrites = 0;
  const get = IDBObjectStore.prototype.get, put = IDBObjectStore.prototype.put;
  IDBObjectStore.prototype.get = function(...args) {if (this.transaction.db.name === name && this.name === "documents") canonicalReads++;return get.apply(this,args);};
  IDBObjectStore.prototype.put = function(...args) {if (this.transaction.db.name === name) {if (this.name === "documents") canonicalWrites++;if (this.name.startsWith("catalog")) projectionWrites++;}return put.apply(this,args);};
  const record = async () => {
    const connection = await raw(name), read = connection.transaction(["catalogPostings",indexedPostingStore],"readonly"), done = catalogTransactionDone(read);
    const rows = await Promise.all(["com","sen"].map(async gram => {
      const row = await catalogRequest(read.objectStore(indexedPostingStore).get(gram));
      return row?.value ?? catalogRequest(read.objectStore("catalogPostings").get(gram));
    }));
    await done;connection.close();return rows;
  };
  try {
    db = await openIndexedDatabase(name);
    check(db.searchNotes,"search port unavailable");
    check((await db.searchNotes({search: "common"})).items.length === 1,"old sparse query");
    check((await record())[0] instanceof Uint32Array,"v4 vector retained");
    check(canonicalReads === 0 && canonicalWrites === 0 && projectionWrites === 0,"upgrade must not eagerly convert prepared rows");
    const read = await db.read({pages: ["legacy-0"],documents: ["legacy-0"]});
    check(read.version === 11 && (read.rows.pages?.["legacy-0"] as NotePage).revision === 17,"upgrade preserves authoring versions");
    check(canonicalReads === 1,"positive body-read control");canonicalReads = 0;
    // Conversion of a previously untouched legacy term must roll back together
    // with its count/authority when a later page in the same save is invalid.
    let legacyRollback = false;
    try {await db.commit(11,[...changes("legacy-0","replacement"),{store: "pages",key: "invalid",value: {...page("invalid"),updatedAt: "invalid"}}]);}
    catch (error) {legacyRollback = error instanceof Error && error.message.includes("note_catalog_timestamp_invalid");}
    check(legacyRollback && (await record())[0] instanceof Uint32Array && (await db.read({})).version === 11
      && (await db.searchNotes({search: "common"})).items.length === 1,"failed legacy conversion removed its original row/count");
    const before = await record();
    await db.commit(11,Array.from({length: 200},(_,i) => changes("new-" + String(i).padStart(3,"0"),"common")).flat());
    const [dense,sparse] = await record();
    check(dense.encoding === "u32-bitmap-v1" && dense.count === 201,"actual save chooses dense representation");
    check(sparse instanceof Uint32Array && sparse.length === 1,"untouched sparse term remains compatible");
    check((await db.searchNotes({search: "common",limit: 500})).items.length === 201 && (await db.searchNotes({search: "sentinel"})).items.length === 1,"mixed query parity");
    check(canonicalReads === 0,"mixed prepared search cannot read canonical bodies");
    let rejected = false;
    try {await db.commit(12,[...changes("rollback-valid","common"),...changes("rollback-invalid","common").map(change => change.store === "documents" ? {...change,value: {damaged: true}} : change)]);} catch {rejected = true;}
    check(rejected && (await db.read({})).version === 12 && (await db.searchNotes({search: "common",limit: 500})).items.length === 201,"failed multi-note write rolls back both representations");
    const broken = await raw(name), corrupt = broken.transaction(indexedPostingStore,"readwrite"), corrupted = catalogTransactionDone(corrupt);
    const bytes = dense.bytes.slice();bytes[0] |= 1;corrupt.objectStore(indexedPostingStore).put({schema: 1,gram: "com",count: dense.count,value: {...dense,bytes}});await corrupted;broken.close();
    let diagnosed = false;
    try {await db.searchNotes({search: "common"});} catch (error) {diagnosed = error instanceof Error && "code" in error && error.code === "index_failed" && error.message.includes("note_search_postings_invalid");}
    check(diagnosed,"corrupt bitmap has a stable error rather than false empty results");
    const repair = await raw(name), restore = repair.transaction(indexedPostingStore,"readwrite"), restored = catalogTransactionDone(restore);
    restore.objectStore(indexedPostingStore).put({schema: 1,gram: "com",count: dense.count,value: dense});await restored;repair.close();
    await db.commit(12,Array.from({length: 190},(_,i) => {const id = "new-" + String(i).padStart(3,"0");return [{store: "pages" as const,key: id},{store: "documents" as const,key: id}];}).flat());
    const after = await record();
    check(after[0] instanceof Uint32Array && after[0].length === 11,"removal returns to sparse representation");
    check((await db.searchNotes({search: "common",limit: 500})).items.length === 11 && (await db.read({})).version === 13,"removal keeps counts and original CAS");
    await db.commit(13,["legacy-0",...Array.from({length: 10},(_,i) => "new-" + String(i + 190).padStart(3,"0"))]
      .flatMap(id => [{store: "pages" as const,key: id},{store: "documents" as const,key: id}]));
    check((await db.searchNotes({search: "common"})).items.length === 0,"deleted indexed row resurrected stale legacy members");
    check(canonicalReads === 0,"all prepared queries avoid canonical bodies");
    const oldVersionRejected = await new Promise<boolean>((resolve,reject) => {
      const request = indexedDB.open(name,4);request.onerror = () => resolve(request.error?.name === "VersionError");request.onsuccess = () => {request.result.close();reject(Error("old writer reopened"));};
    });
    return {oldClosed,oldVersionRejected,oldSparse: before[0] instanceof Uint32Array,denseBytes: dense.bytes.byteLength,sparseBytes: after[0].byteLength,canonicalReads,diagnosed};
  } finally {
    IDBObjectStore.prototype.get = get;IDBObjectStore.prototype.put = put;db?.close?.();old.close();
    await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});
  }
}
