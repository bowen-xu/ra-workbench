import {createNoteDocumentFromText,notePageId,noteBlockId,type NotePage} from "@research-assistant/core/notes";
import {UnifiedNoteCatalog} from "@research-assistant/data/notes";
import {RetainedNoteCatalog} from "../../apps/web/src/core/features/notes/workspace/RetainedNoteCatalog";
import {openIndexedDatabase} from "../../apps/web/src/core/features/notes/gateways/browserNotesDatabase";
import {catalogRequest,catalogTransactionDone,type BrowserCatalogState} from "../../apps/web/src/core/features/notes/gateways/browserNoteCatalogStore";

const recent = {category: "independent",view: "recent",search: "",limit: 20} as const;
function check(value: unknown, message: string): asserts value {if (!value) throw Error(message);}
async function connect(name: string) {
  return new Promise<IDBDatabase>((resolve,reject) => {
    const request = indexedDB.open(name);request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);
  });
}
async function progress(db: IDBDatabase) {
  const tx = db.transaction(["catalogState","operations","version"],"readonly"), done = catalogTransactionDone(tx);
  const [state,version,hbs] = await Promise.all([
    catalogRequest(tx.objectStore("catalogState").get("current")),
    catalogRequest(tx.objectStore("version").get("current")),
    catalogRequest(tx.objectStore("operations").get("opaque-hbs")),
  ]);
  await done;return {state: state as BrowserCatalogState,version,hbs};
}

/** Real IndexedDB preparation plus production retained query owner; cancellation
 * is triggered while a document request is dispatched, not by a mocked promise. */
export async function verifyCatalogCancellation(sharedConsumer: boolean) {
  const name = "note-catalog-cancel-" + crypto.randomUUID(), db = await openIndexedDatabase(name);
  const raw = await connect(name), reads = new Map<string,number>();
  const get = IDBObjectStore.prototype.get;
  let interrupt: (() => void) | undefined;
  IDBObjectStore.prototype.get = function(...args) {
    if (this.transaction.db.name === name && this.name === "documents") {
      const key = String(args[0]);reads.set(key,(reads.get(key) ?? 0) + 1);
      const callback = interrupt;interrupt = undefined;callback?.();
    }
    return get.apply(this,args);
  };
  const source = new UnifiedNoteCatalog({queryNotePageCatalog: (query,options) => db.queryCatalog!({...query,search: query.search ?? ""},options)},
    {queryWritingNoteCatalog: async () => ({items: [],revision: 1,pendingCount: 0,errorCount: 0,issues: []})},
    async () => {throw Error("single-source query loaded collation");});
  const retained = new RetainedNoteCatalog(source);
  try {
    const changes = Array.from({length: 48},(_,index) => {
      const id = notePageId("page-" + String(index).padStart(3,"0")), timestamp = new Date(Date.UTC(2026,9,24) + index * 1000).toISOString();
      const page: NotePage = {id,title: "Title " + index,position: index,status: "active",schemaVersion: 1,revision: 0,isFavorite: false,
        projectIds: [],tagIds: [],createdAt: timestamp,updatedAt: timestamp};
      return [{store: "pages" as const,key: id,value: page},{store: "documents" as const,key: id,
        value: createNoteDocumentFromText(id,`needle-${index}`,() => noteBlockId("block-" + index))}];
    }).flat();
    await db.commit(0,[...changes,{store: "operations",key: "opaque-hbs",value: {owner: "unchanged"}}]);
    check((await retained.read(recent,1,0)).items.length === 20 && reads.size === 0,"recent page hydrated documents");
    const controller = new AbortController();
    interrupt = sharedConsumer ? () => controller.abort() : () => retained.suspend();
    const search = {...recent,search: "needle-47"};
    const request = sharedConsumer ? db.queryCatalog!(search,{signal: controller.signal}) : retained.read(search,1,0);
    const canceled = request.then(() => false, error => error.name === "AbortError");
    const live = sharedConsumer ? db.queryCatalog!(search) : undefined;
    check(await canceled,"canceled consumer received a completed query");
    const first = await progress(raw);
    if (!sharedConsumer) {
      check(reads.size === 8 && first.state.bodyScopes?.active?.after === "page-007" && !first.state.searchReady,"cancel did not retain exactly one committed batch");
      await new Promise(resolve => setTimeout(resolve,50));
      check(reads.size === 8,"hidden owner continued body preparation");
      await retained.read(recent,1,0);
      check(reads.size === 8,"cached recent page resumed search preparation");
      // An independent canonical save must remain usable while preparation rests.
      const pendingPage = changes.find(change => change.store === "pages" && change.key === "page-030")!.value as NotePage;
      await db.commit(1,[{store: "documents",key: "page-000",value: createNoteDocumentFromText(notePageId("page-000"),"edited-needle",() => noteBlockId("edit"))},
        {store: "pages",key: "page-030",value: {...pendingPage,title: "Renamed while paused",revision: 1}}]);
      check(reads.size === 8,"save reread indexed canonical body");
    }
    const result = await (live ?? retained.read(search,2,0));
    check(result.items.length === 1 && result.items[0]?.id === "page:page-047","resumed/live search lost its result");
    if (!sharedConsumer) check((await db.queryCatalog!({...recent,search: "Renamed while paused"})).items[0]?.title === "Renamed while paused","resumed preparation reused metadata from an earlier transaction");
    check(reads.size === 48 && [...reads.values()].every(count => count === 1),"resumption or shared work repeated canonical bodies");
    const final = await progress(raw);
    check(final.state.bodyScopes?.active?.ready && final.state.bodyScopes?.archived?.ready && !final.state.bodyScopes?.trashed?.ready && final.version === (sharedConsumer ? 1 : 2) && (final.hbs as {owner: string}).owner === "unchanged","preparation changed canonical version/HBS or did not finish");
    const beforeHot = [...reads.values()].reduce((sum,count) => sum + count,0);
    await db.queryCatalog!(search);
    check([...reads.values()].reduce((sum,count) => sum + count,0) === beforeHot,"ready search reread canonical bodies");
    const aborted = new AbortController();aborted.abort();
    check(await db.queryCatalog!(search,{signal: aborted.signal}).then(() => false,error => error.name === "AbortError"),"pre-aborted request dispatched");
    return {sharedConsumer,firstBatch: sharedConsumer ? undefined : 8,totalBodyReads: beforeHot,repeatReads: 0,version: final.version};
  } finally {
    retained.dispose();IDBObjectStore.prototype.get = get;raw.close();db.close?.();
    await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);});
  }
}
