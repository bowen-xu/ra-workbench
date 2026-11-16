import {createNoteDocumentFromText,notePageId,noteBlockId,type NotePage,type NoteReadingSource} from "@research-assistant/core/notes";
import {openIndexedDatabase,noteStores,type NoteWrite} from "../../apps/web/src/core/features/notes/gateways/browserNotesDatabase";
import {ensureBrowserNoteCatalog} from "../../apps/web/src/core/features/notes/gateways/browserNoteCatalogOwner";
import {browserCatalogStores,createCatalogStores,catalogRequest,catalogTransactionDone,type BrowserCatalogState} from "../../apps/web/src/core/features/notes/gateways/browserNoteCatalogStore";
import {createIndexedPostingStore} from "../../apps/web/src/core/features/notes/gateways/browserCatalogPostingRows";
import {loadBrowserNoteCatalogRuntime} from "../../apps/web/src/core/features/notes/gateways/browserNoteCatalogRuntime";
import {applyBrowserNoteCatalogChanges} from "../../apps/web/src/core/features/notes/gateways/browserNoteCatalogWrites";

function check(value: unknown,message: string): asserts value {if (!value) throw Error(message);}
async function connect(name: string) {
  return new Promise<IDBDatabase>((resolve,reject) => {const request = indexedDB.open(name);request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);});
}
const recent = {category: "independent",view: "recent",search: "",limit: 20} as const;
function seedPage(id: string,status: NotePage["status"],reading = false): NoteWrite[] {
  const page: NotePage = {id: notePageId(id),title: id,position: 0,status,schemaVersion: 1,revision: 0,isFavorite: false,projectIds: [],tagIds: [],
    createdAt: "2026-10-24T00:00:00Z",updatedAt: "2026-10-24T00:00:00Z"};
  const result: NoteWrite[] = [{store: "pages",key: id,value: page},{store: "documents",key: id,
    value: createNoteDocumentFromText(page.id,`needle ${id}`,() => noteBlockId("block-" + id))}];
  if (reading) {
    const source: NoteReadingSource = {id: noteBlockId("reading-" + id),pageId: page.id,noteType: "highlight",color: "green",
      source: {paperId: "paper",paperTitle: "Paper",attachmentId: "pdf",contentSha256: "a".repeat(64),pageIndex: 0,rects: [],quoteExact: "quote",quotePrefix: "",quoteSuffix: "",state: "exact"}};
    result.push({store: "sources",key: source.id,value: source});
  }
  return result;
}
async function readState(raw: IDBDatabase) {
  const tx = raw.transaction(["catalogState","version","operations"],"readonly"), done = catalogTransactionDone(tx);
  const [state,version,hbs] = await Promise.all([catalogRequest(tx.objectStore("catalogState").get("current")),catalogRequest(tx.objectStore("version").get("current")),catalogRequest(tx.objectStore("operations").get("opaque-hbs"))]);
  await done;return {state: state as BrowserCatalogState,version,hbs};
}

/** Exercise real owner/index ranges and failures outside the requested scope.
 * No mock search or pre-marked readiness is used. */
export async function verifyCatalogScopes() {
  const name = "note-scopes-" + crypto.randomUUID();let db = await openIndexedDatabase(name);
  const raw = await connect(name), reads: string[] = [], get = IDBObjectStore.prototype.get;
  IDBObjectStore.prototype.get = function(...args) {if (this.transaction.db.name === name && this.name === "documents") reads.push(String(args[0]));return get.apply(this,args);};
  try {
    const changes = ["active","archived","trashed"].flatMap(status => Array.from({length: 16},(_,i) => seedPage(`${status}-${String(i).padStart(2,"0")}`,status as NotePage["status"],i < 2)).flat());
    await db.commit(0,[...changes,{store: "operations",key: "opaque-hbs",value: {owner: "unchanged"}}]);
    const first = await db.queryCatalog!(recent);
    check(first.items.length === 14 && reads.length === 0,"first independent list loaded bodies");
    await db.queryCatalog!({...recent,category: "all",sort: {field: "title",direction: "ascending"}});
    check(reads.length === 2 && reads.every(id => id.startsWith("active-0")),"title sort read ordinary or other-status bodies");
    await db.searchNotes!({search: "needle"});
    check(reads.length === 16 && reads.every(id => id.startsWith("active-")),"active object search prepared unrelated states");
    let progress = await readState(raw);
    check(progress.state.bodyScopes?.active?.ready && !progress.state.bodyScopes?.archived?.ready && !progress.state.searchReady,"one scope marked all scopes ready");
    await db.queryCatalog!({...recent,category: "all",search: "needle"});
    check(reads.length === 32 && !reads.some(id => id.startsWith("trashed-")),"live main search must prepare active and archived only");
    const liveReads = reads.length;
    await db.queryCatalog!({...recent,search: "needle"});
    db.close?.();db = await openIndexedDatabase(name);
    await db.searchNotes!({search: "needle"});
    check(reads.length === liveReads,"hot search/reopen read canonical bodies again");
    await db.queryCatalog!({...recent,view: "trash",category: "all",sort: {field: "title",direction: "ascending"}});
    check(reads.length === 34 && reads.slice(-2).every(id => id.startsWith("trashed-0")),"trash titles crossed lifecycle scope");
    await db.searchNotes!({search: "needle",status: "trashed"});
    check(reads.length === 48 && new Set(reads).size === 48,"later trash preparation repeated canonical bodies");
    progress = await readState(raw);
    check(progress.state.searchReady && progress.version === 1 && (progress.hbs as {owner: string}).owner === "unchanged","derived preparation changed authoring/HBS");
    await db.commit(1,[{store: "documents",key: "trashed-15"}]);
    check((await db.searchNotes!({search: "needle"})).items.length === 16 && reads.length === 48,"broken trash scope blocked or rehydrated active search");
    check(await db.searchNotes!({search: "needle",status: "trashed"}).then(() => false,error => error.code === "index_failed" && error.message.includes("note_catalog_document_missing") && error.message.includes("trashed-15")),"missing requested body lost its diagnostic");
    return {firstListBodies: 0,activeTitleBodies: 2,activeSearchBodies: 16,liveSearchBodies: 32,hotOrReopenBodies: 0,allScopesBodies: 48};
  } finally {
    IDBObjectStore.prototype.get = get;raw.close();db.close?.();
    await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});
  }
}

/** Move an unindexed identity behind a saved cursor, first while paused and
 * then while the other live scope is preparing. Final snapshot readiness must
 * retry the invalidated scope; authoring still has exactly its original CAS. */
export async function verifyCatalogScopeMoves(duringOtherScope: boolean) {
  const name = "note-scope-move-" + crypto.randomUUID(), db = await openIndexedDatabase(name), raw = await connect(name);
  const reads: string[] = [], get = IDBObjectStore.prototype.get;
  let interrupt: (() => void) | undefined;
  IDBObjectStore.prototype.get = function(...args) {
    if (this.transaction.db.name === name && this.name === "documents") {
      reads.push(String(args[0]));const callback = interrupt;interrupt = undefined;callback?.();
    }
    return get.apply(this,args);
  };
  try {
    const moved = seedPage("000-moved","trashed"), changes = [...moved,...Array.from({length: 24},(_,i) => seedPage(`active-${String(i).padStart(2,"0")}`,"active")).flat(),...seedPage("archive-body","archived")];
    await db.commit(0,[...changes,{store: "operations",key: "opaque-hbs",value: {owner: "unchanged"}}]);
    await db.queryCatalog!(recent);
    // Prepare this writer's lazy runtime before the race. Otherwise its first
    // transaction intentionally aborts for runtime loading and the move may
    // legitimately commit only after the search's readonly snapshot.
    if (duringOtherScope) await db.commit(1,[changes.find(change => change.store === "pages" && change.key === "active-00")!]);
    const version = duringOtherScope ? 2 : 1;
    const page = moved[0]!.value as NotePage;
    const move = () => db.commit(version,[{store: "pages",key: page.id,value: {...page,status: "active",revision: 1}}]);
    if (duringOtherScope) {
      await ensureBrowserNoteCatalog(raw,"search",undefined,["active"]);
      let moving: Promise<void> | undefined;
      interrupt = () => {moving = move();};
      const found = await db.queryCatalog!({...recent,search: "000-moved"});
      await moving;
      check(found.items.length === 1 && found.items[0]?.id === "page:000-moved","scope invalidated during other scope preparation was not retried");
    } else {
      const controller = new AbortController();interrupt = () => controller.abort();
      check(await ensureBrowserNoteCatalog(raw,"search",controller.signal,["active"]).then(() => false,error => error.name === "AbortError"),"preparation did not pause");
      const paused = await readState(raw);
      check(paused.state.bodyScopes?.active?.after === "active-07" && reads.length === 8,"wrong committed pause cursor");
      await move();
      check(!(await readState(raw)).state.bodyScopes?.active?.after,"status move failed to reset cursor");
      const found = await db.searchNotes!({search: "000-moved"});
      check(found.items.length === 1 && found.items[0]?.objectId === "000-moved","resumed status scope skipped moved identity");
    }
    check(new Set(reads).size === reads.length && reads.includes("000-moved"),"status invalidation reread prepared bodies");
    const final = await readState(raw);
    check(final.version === version + 1 && (final.hbs as {owner: string}).owner === "unchanged","status preparation changed canonical/HBS");
    return {duringOtherScope,bodyReads: reads.length,movedReads: reads.filter(id => id === "000-moved").length,version: final.version};
  } finally {
    IDBObjectStore.prototype.get = get;raw.close();db.close?.();
    await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});
  }
}

/** Frozen v7 shape: existing indexed posting rows and global readiness, without
 * either v8 range index or scoped state. Upgrade never reads/rewrites bodies. */
export async function verifyCatalogScopeUpgrade(complete = true) {
  const name = "note-scope-v7-" + crypto.randomUUID();
  const legacy = await new Promise<IDBDatabase>((resolve,reject) => {
    const request = indexedDB.open(name,7);
    request.onupgradeneeded = () => {
      for (const store of [...noteStores,"version"]) request.result.createObjectStore(store);
      createCatalogStores(request.result);createIndexedPostingStore(request.result);
      request.transaction!.objectStore("sources").createIndex("pageId","pageId");
      request.transaction!.objectStore("links").createIndex("sourcePageId","sourcePageId");
      request.transaction!.objectStore("catalogHeaders").createIndex("reading",["item.target.kind","item.id"]);
    };
    request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);
  });
  const runtime = await loadBrowserNoteCatalogRuntime(), changes = [seedPage("active-old","active"),seedPage("archive-old","archived"),...(!complete ? [seedPage("active-new","active")] : [])].flat();
  const tx = legacy.transaction([...browserCatalogStores,...noteStores,"version"],"readwrite"), done = catalogTransactionDone(tx);
  await applyBrowserNoteCatalogChanges(tx,changes.filter(change => change.store !== "documents" || change.key !== "active-new"),runtime);
  for (const change of changes) tx.objectStore(change.store).put(change.value,change.key);
  const state = await catalogRequest(tx.objectStore("catalogState").get("current")) as BrowserCatalogState;
  const {bodyScopes: _scopes,readingTitleScopes: _titles,...deployed} = state;
  tx.objectStore("catalogState").put({...deployed,headersReady: true,searchReady: complete,readingTitlesReady: complete,searchMetadataReady: true,...(!complete ? {bodyCursor: "zzzz-old-global-cursor"} : {})},"current");
  tx.objectStore("version").put(1,"current");await done;
  let closed = false, upgradeReads = 0, bodyReads = 0, upgrading = true;
  legacy.onversionchange = () => {closed = true;legacy.close();};
  const get = IDBObjectStore.prototype.get;
  IDBObjectStore.prototype.get = function(...args) {
    if (this.transaction.db.name === name) {if (upgrading) upgradeReads++;if (this.name === "documents") bodyReads++;}
    return get.apply(this,args);
  };
  let db: Awaited<ReturnType<typeof openIndexedDatabase>> | undefined;
  try {
    db = await openIndexedDatabase(name);upgrading = false;
    check(closed && upgradeReads === 0 && (await db.searchNotes!({search: "needle"})).items.length === (complete ? 1 : 2),"v7 global progress hid an unindexed active identity");
    check(bodyReads === (complete ? 0 : 1),"v7 query did not reuse existing bodies");
    // Introducing one unindexed active page cannot invalidate the other scopes.
    const added = seedPage("active-later","active");
    await db.commit(1,[added[0]!]);
    check((await db.searchNotes!({search: "needle",status: "archived"})).items.length === 1 && bodyReads === (complete ? 0 : 1),"active invalidation reloaded indexed archive bodies");
    const raw = await connect(name), progress = await readState(raw);raw.close();
    check(!progress.state.searchReady && progress.state.bodyScopes?.archived?.ready && progress.version === 2,"v7 completion expansion was not persisted");
    return {closed,upgradeReads,bodyReads,version: progress.version};
  } finally {
    IDBObjectStore.prototype.get = get;legacy.close();db?.close?.();
    await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});
  }
}
