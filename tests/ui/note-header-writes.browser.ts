import {createNoteDocumentFromText,decodeNoteDocument,noteDocumentPlainText,notePageId,noteBlockId,type NotePage,type NoteReadingSource} from "@research-assistant/core/notes";
import {openIndexedDatabase} from "../../apps/web/src/core/features/notes/gateways/browserNotesDatabase";
import {catalogRequest,catalogTransactionDone} from "../../apps/web/src/core/features/notes/gateways/browserNoteCatalogStore";

function check(value: unknown,message: string): asserts value {if (!value) throw Error(message);}
export async function verifyHeaderWrites() {
  const name = "note-header-writes-" + crypto.randomUUID(), db = await openIndexedDatabase(name);
  const id = notePageId("ordinary"), timestamp = "2026-10-24T00:00:00Z";
  const page: NotePage = {id,title: "Title",position: 0,status: "active",schemaVersion: 1,revision: 0,isFavorite: false,
    projectIds: [],tagIds: [],createdAt: timestamp,updatedAt: timestamp};
  const body = (text: string) => createNoteDocumentFromText(id,text,() => noteBlockId(crypto.randomUUID()));
  const query = {category: "all",view: "recent",search: "",limit: 20} as const;
  const source: NoteReadingSource = {id: noteBlockId("reading"),pageId: id,noteType: "highlight",color: "green",source: {
    paperId: "paper",paperTitle: "Paper",attachmentId: "pdf",contentSha256: "",pageIndex: 0,rects: [],quoteExact: "Quote fallback",quotePrefix: "",quoteSuffix: "",state: "exact",
  }};
  let headerWrites = 0, canonicalReads = 0, projectionReads = 0;
  const put = IDBObjectStore.prototype.put, get = IDBObjectStore.prototype.get;
  IDBObjectStore.prototype.put = function(...args) {if (this.transaction.db.name === name && this.name === "catalogHeaders") headerWrites++;return put.apply(this,args);};
  IDBObjectStore.prototype.get = function(...args) {
    if (this.transaction.db.name === name) {if (this.name === "documents") canonicalReads++;if (this.name === "catalogBodies") projectionReads++;}
    return get.apply(this,args);
  };
  const reset = () => {headerWrites = 0;canonicalReads = 0;projectionReads = 0;};
  try {
    await db.commit(0,[{store: "pages",key: id,value: page},{store: "documents",key: id,value: body("Old line\nold detail")},
      {store: "operations",key: "opaque-hbs",value: {owner: "unchanged"}}]);
    await db.queryCatalog!(query);reset();
    await db.queryCatalog!({...query,search: "old detail"});
    check(headerWrites === 0 && canonicalReads === 1,"ordinary cold body preparation rewrote its sorting header");
    const cold = {headerWrites,canonicalReads};reset();
    await db.commit(1,[{store: "documents",key: id,value: body("Latest line\nnew detail")}]);
    check(headerWrites === 0 && canonicalReads === 0,"ordinary body save rewrote unchanged metadata or reread body");
    // Case-only display changes must survive even if folded search text matches.
    await db.commit(2,[{store: "pages",key: id,value: {...page,title: "TITLE",revision: 1}}]);
    check(headerWrites === 1 && (await db.queryCatalog!(query)).items[0]?.title === "TITLE","case-only metadata change was treated as no-op");
    reset();
    await db.commit(3,[{store: "sources",key: source.id,value: source}]);
    check(headerWrites === 1 && canonicalReads === 0 && projectionReads === 1,"source attachment did not use the current persisted body exactly once");
    const attached = await db.queryCatalog!(query);
    check(attached.items[0]?.title === "Latest line" && attached.items[0]?.target.kind === "reading","source attachment reused stale/empty ordinary title cache");
    reset();
    await db.commit(4,[{store: "documents",key: id,value: body("Latest line\nchanged tail")}]);
    check(headerWrites === 0 && canonicalReads === 0,"reading tail edit rewrote its unchanged sorting header");
    await db.commit(5,[{store: "documents",key: id,value: body("Next title\nchanged tail")}]);
    check(headerWrites === 1 && (await db.queryCatalog!(query)).items[0]?.title === "Next title","reading title change was not reindexed");
    check((await db.queryCatalog!({...query,search: "Latest line"})).items.length === 0,"old body/title membership survived");
    check((await db.queryCatalog!({...query,search: "Next title"})).items.length === 1,"new body/title membership missing");
    reset();
    await db.commit(6,[{store: "sources",key: source.id}]);
    check((await db.queryCatalog!(query)).items[0]?.title === "TITLE","detach did not restore canonical ordinary title");
    const detachedWrites = headerWrites;reset();
    await db.commit(7,[{store: "documents",key: id,value: body("Detached latest\ntext")}]);
    check(headerWrites === 0,"detached ordinary body rewrote the indexed header");
    await db.commit(8,[{store: "sources",key: source.id,value: source}]);
    check((await db.queryCatalog!(query)).items[0]?.title === "Detached latest","reattachment did not use current body");
    reset();
    const updatedAt = "2026-10-24T00:00:01Z";
    await db.commit(9,[{store: "pages",key: id,value: {...page,title: "TITLE",revision: 2,updatedAt}},
      {store: "documents",key: id,value: body("Detached latest\nsaved with timestamp")}]);
    check(headerWrites === 1 && canonicalReads === 0 && (await db.queryCatalog!(query)).items[0]?.updatedAt === updatedAt,"body plus timestamp save lost its indexed display update");
    // A failed mixed save must retain the original authority/projection boundary.
    let rejected = false;
    try {await db.commit(10,[{store: "documents",key: id,value: body("Must roll back")},{store: "pages",key: "broken",value: {...page,id: notePageId("broken"),updatedAt: "invalid"}}]);} catch (reason) {rejected = reason instanceof Error && reason.message.includes("note_catalog_timestamp_invalid");}
    check(rejected && (await db.queryCatalog!(query)).items[0]?.title === "Detached latest","mixed save did not roll back");
    const final = await db.read({operations: ["opaque-hbs"],documents: [id]});
    check(noteDocumentPlainText(decodeNoteDocument(final.rows.documents?.[id])) === "Detached latest\nsaved with timestamp","failed save leaked its canonical body");
    check(final.version === 10 && (final.rows.operations?.["opaque-hbs"] as {owner: string}).owner === "unchanged","projection optimization changed canonical CAS/HBS");
    return {cold,detachedWrites,version: final.version,rolledBack: rejected};
  } finally {
    IDBObjectStore.prototype.put = put;IDBObjectStore.prototype.get = get;db.close?.();
    await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});
  }
}

/** A prepared v5 cache must upgrade without reading/writing bodies, and cannot
 * remain writable by old code that expects ordinary first-line caches. */
export async function verifyHeaderUpgrade() {
  const name = "note-header-upgrade-" + crypto.randomUUID();
  const {noteStores} = await import("../../apps/web/src/core/features/notes/gateways/browserNotesDatabase");
  const {createCatalogStores,catalogStores} = await import("../../apps/web/src/core/features/notes/gateways/browserNoteCatalogStore");
  const old = await new Promise<IDBDatabase>((resolve,reject) => {
    const request = indexedDB.open(name,5);request.onupgradeneeded = () => {
      for (const store of [...noteStores,"version"]) request.result.createObjectStore(store);
      request.transaction!.objectStore("sources").createIndex("pageId","pageId");
      request.transaction!.objectStore("links").createIndex("sourcePageId","sourcePageId");
      createCatalogStores(request.result);request.transaction!.objectStore("catalogHeaders").createIndex("reading",["item.target.kind","item.id"]);
    };request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);
  });
  const {loadBrowserNoteCatalogRuntime} = await import("../../apps/web/src/core/features/notes/gateways/browserNoteCatalogRuntime");
  const {applyBrowserNoteCatalogChanges} = await import("../../apps/web/src/core/features/notes/gateways/browserNoteCatalogWrites");
  const runtime = await loadBrowserNoteCatalogRuntime(), id = notePageId("legacy"), timestamp = "2026-10-24T00:00:00Z";
  const page: NotePage = {id,title: "Legacy",position: 0,status: "active",schemaVersion: 1,revision: 17,isFavorite: false,
    projectIds: [],tagIds: [],createdAt: timestamp,updatedAt: timestamp};
  const changes = [{store: "pages" as const,key: id,value: page},{store: "documents" as const,key: id,
    value: createNoteDocumentFromText(id,"Stored body",() => noteBlockId("legacy-block"))}];
  const tx = old.transaction([...noteStores,...catalogStores,"version"],"readwrite"), done = catalogTransactionDone(tx);
  await applyBrowserNoteCatalogChanges(tx,changes,runtime);
  for (const change of changes) tx.objectStore(change.store).put(change.value,change.key);
  const header = await catalogRequest(tx.objectStore("catalogHeaders").get("page:legacy"));
  // Restore the deployed v5 ordinary-cache shape, with complete valid postings.
  tx.objectStore("catalogHeaders").put({...header,bodyFirstLine: "Stored body"});
  const state = await catalogRequest(tx.objectStore("catalogState").get("current"));
  tx.objectStore("catalogState").put({...state,headersReady: true,searchReady: true,readingTitlesReady: true,searchMetadataReady: true},"current");
  tx.objectStore("version").put(17,"current");tx.objectStore("operations").put({owner: "unchanged"},"opaque-hbs");await done;
  let closed = false;old.onversionchange = () => {closed = true;old.close();};
  let reads = 0,writes = 0;
  const get = IDBObjectStore.prototype.get,put = IDBObjectStore.prototype.put;
  IDBObjectStore.prototype.get = function(...args) {if (this.transaction.db.name === name) reads++;return get.apply(this,args);};
  IDBObjectStore.prototype.put = function(...args) {if (this.transaction.db.name === name) writes++;return put.apply(this,args);};
  let db: Awaited<ReturnType<typeof openIndexedDatabase>> | undefined;
  try {
    db = await openIndexedDatabase(name);check(closed && reads === 0 && writes === 0,"v6 upgrade scanned/rebuilt records or left old writer open");
    const rejected = await new Promise<boolean>((resolve,reject) => {const request = indexedDB.open(name,5);request.onerror = () => resolve(request.error?.name === "VersionError");request.onsuccess = () => {request.result.close();reject(Error("old v5 writer reopened"));};});
    check((await db.queryCatalog!({category: "all",view: "recent",search: "Stored body",limit: 20})).items[0]?.title === "Legacy" && writes === 0,"prepared v5 search was rebuilt or lost its ordinary title");
    const state = await db.read({operations: ["opaque-hbs"]});
    check(rejected && state.version === 17 && (state.rows.operations?.["opaque-hbs"] as {owner: string}).owner === "unchanged","upgrade changed CAS/HBS or accepted old writer");
    return {closed,rejected,upgradeReads: 0,upgradeWrites: 0,version: state.version};
  } finally {
    IDBObjectStore.prototype.get = get;IDBObjectStore.prototype.put = put;old.close();db?.close?.();
    await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});
  }
}
