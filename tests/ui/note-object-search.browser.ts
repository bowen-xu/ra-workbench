import {createNoteDocumentFromText,notePageId,noteBlockId,type NotePage,type NoteReadingSource} from "@research-assistant/core/notes";
import type {NoteSearchQuery} from "@research-assistant/data/notes";
import {openIndexedDatabase,type BrowserNotesDatabase,type NoteWrite} from "../../apps/web/src/core/features/notes/gateways/browserNotesDatabase";
import {catalogRequest,catalogTransactionDone,type BrowserCatalogState} from "../../apps/web/src/core/features/notes/gateways/browserNoteCatalogStore";

type Metrics = {readonly reads: Record<string,number>;readonly rows: Record<string,number>;readonly keys: Record<string,number>;readonly writes: Record<string,number>};
function instrument(name: string) {
  const metrics: Metrics = {reads: {},rows: {},keys: {},writes: {}};
  const originals: (() => void)[] = [];
  let rejectDocuments = false, failBatch = false;
  for (const [target,methods] of [[IDBObjectStore.prototype,["get","getAll","getKey","openCursor","put","delete"]],[IDBIndex.prototype,["get","getKey","getAll","openCursor","openKeyCursor"]]] as const) {
    const prototype = target as unknown as Record<string,(...args: unknown[]) => IDBRequest<unknown>>;
    for (const method of methods) {
      const original = prototype[method]!;
      prototype[method] = function(this: IDBObjectStore | IDBIndex,...args: unknown[]) {
        const store = this instanceof IDBIndex ? this.objectStore : this;
        if (store.transaction.db.name === name) {
          if (rejectDocuments && store.name === "documents" && !["put","delete"].includes(method)) throw Error("canonical body was read after readiness");
          if (failBatch && store.name === "catalogState" && method === "put" && (args[0] as BrowserCatalogState).searchMetadataCursor === "page-0199") {
            failBatch = false; throw Error("synthetic metadata second batch failure");
          }
        }
        const request = Reflect.apply(original,this,args) as IDBRequest<unknown>;
        if (store.transaction.db.name !== name) return request;
        const group = ["put","delete"].includes(method) ? metrics.writes : metrics.reads;
        group[store.name] = (group[store.name] ?? 0) + 1;
        if (group === metrics.reads) request.addEventListener("success",() => {
          if (request.result === undefined || request.result === null) return;
          const returned = method === "getKey" || method === "openKeyCursor" ? metrics.keys : metrics.rows;
          returned[store.name] = (returned[store.name] ?? 0) + (method === "getAll" && Array.isArray(request.result) ? request.result.length : 1);
        });
        return request;
      };
      originals.push(() => {prototype[method] = original;});
    }
  }
  return {metrics,reset: () => {for (const group of Object.values(metrics)) for (const key of Object.keys(group)) delete group[key];},
    forbidBodies: () => {rejectDocuments = true;},failSecondBatch: () => {failBatch = true;},restore: () => {for (const restore of originals) restore();}};
}
function check(value: unknown,message: string): asserts value {if (!value) throw Error(message);}
function copy(metrics: Metrics): Metrics {return structuredClone(metrics);}
async function raw(name: string) {
  return new Promise<IDBDatabase>((resolve,reject) => {const request = indexedDB.open(name);request.onsuccess = () => resolve(request.result);request.onerror = () => reject(request.error);});
}

/** Runs the production IndexedDB owner, not a memory search implementation. */
export async function verifyNoteObjectSearch() {
  const name = "note-object-search-" + crypto.randomUUID(), probe = instrument(name);
  let db: BrowserNotesDatabase | undefined, other: BrowserNotesDatabase | undefined;
  const query = (input: NoteSearchQuery) => {if (!db?.searchNotes) throw Error("search port missing");return db.searchNotes(input);};
  try {
    db = await openIndexedDatabase(name);
    const changes: NoteWrite[] = [];
    for (let i = 0; i < 2400; i++) {
      const id = notePageId("page-" + String(i).padStart(4,"0")), timestamp = new Date(Date.UTC(2026,9,24) + i * 1000).toISOString();
      const page: NotePage = {id,title: "Title " + i,position: i,status: ["active","archived","trashed"][i % 3] as NotePage["status"],schemaVersion: 1,
        revision: 0,isFavorite: i % 6 === 0,projectIds: i % 2 ? [] : ["project"],tagIds: i % 2 ? [] : ["tag"],createdAt: timestamp,updatedAt: timestamp};
      const text = i === 1737 ? "unique-needle-1737" : `${i === 9 || i === 18 ? "needle " : ""}body-${i}`;
      changes.push({store: "pages",key: id,value: page},{store: "documents",key: id,value: createNoteDocumentFromText(id,text,() => noteBlockId("block-" + i))});
      if (i < 12) {
        const source: NoteReadingSource = {id: noteBlockId("reading-" + String(i).padStart(2,"0")),pageId: id,noteType: "highlight",color: "green",
          source: {paperId: i % 2 ? "other" : "paper",paperTitle: "Synthetic paper title",attachmentId: "pdf",contentSha256: "a".repeat(64),pageIndex: i,rects: [],
            quoteExact: "quote-" + i,quotePrefix: "",quoteSuffix: "",state: "exact"}};
        changes.push({store: "sources",key: source.id,value: source});
      }
    }
    await db.commit(0,changes);
    console.info("[note-object-stage] 2400 canonical records seeded");
    const list = await db.queryCatalog!({category: "all",view: "recent",search: "",limit: 20});
    check(list.items.length === 20,"first main list");
    console.info("[note-object-stage] 2400 headers prepared");
    probe.reset();
    const first = await query({limit: 20}), empty = copy(probe.metrics);
    check(first.items.length === 20 && first.items[0]?.objectId === "page-0012","bounded object order");
    check(!empty.rows.documents && !empty.rows.catalogBodies && empty.rows.catalogHeaders === 21 && empty.keys.catalogHeaders === 21,"empty search must read 21 headers and no bodies: " + JSON.stringify(empty));
    probe.reset();
    const filtered = await query({objectKinds: ["reading_note"],paperId: "paper",projectId: "project",tagId: "tag",isFavorite: true,noteType: "highlight",sourceState: "exact",cursor: JSON.stringify(["reading-00","reading_note"]),limit: 1});
    check(filtered.items[0]?.objectId === "reading-06" && !filtered.nextCursor,"filters before pagination, canonical favorite and independent reading ID");
    check(!probe.metrics.rows.documents && !probe.metrics.rows.catalogBodies,"source filtering cannot hydrate bodies");
    probe.reset();
    const initial = await query({search: "unique-needle-1737",limit: 20}), preparation = copy(probe.metrics);
    console.info("[note-object-stage] 2400 search projection prepared");
    check(initial.items.length === 1 && initial.items[0]?.objectId === "page-1737","first persistent search");
    check(preparation.rows.documents === 800,"one-time body backfill is separate from listing");
    check(preparation.rows.pages === 801,"body preparation must reuse cursor metadata; only the final hit needs a page lookup: " + JSON.stringify(preparation));
    check(preparation.reads.catalogBodies === 801,"confirmed missing projections must not be fetched again inside the same transaction: " + JSON.stringify(preparation));
    probe.forbidBodies(); probe.reset();
    const again = await query({search: "unique-needle-1737",limit: 20}), rare = copy(probe.metrics);
    check(again.items.length === 1 && rare.rows.catalogHeaders === 1 && rare.keys.catalogHeaders === 1 && rare.rows.catalogBodies === 1 && !rare.rows.documents,"ready rare query bounded candidate reads: " + JSON.stringify(rare));
    const page1 = await query({search: "needle",limit: 1});
    check(page1.items[0]?.objectId === "page-0018" && page1.items[0]?.snippet === "needle body-18","plain text rather than serialized JSON");
    const page2 = await query({search: "needle",limit: 1,cursor: page1.nextCursor});
    check(page2.items[0]?.objectId === "page-1737","mixed object pagination second ordinary");
    const page3 = await query({search: "needle",limit: 1,cursor: page2.nextCursor});
    check(page3.items[0]?.objectId === "reading-09" && !page3.nextCursor,"mixed object pagination reading alias");
    check((await query({search: "Synthetic paper title",objectKinds: ["reading_note"]})).items.length === 4,"reading paper title included without origin links");
    check((await query({search: '"kind":"inline"'})).items.length === 0,"storage JSON cannot create false text hits");
    other = await openIndexedDatabase(name);
    const version = (await other.read({})).version;
    probe.reset();
    await other.commit(version,[{store: "documents",key: "page-0009",value: createNoteDocumentFromText(notePageId("page-0009"),"external update",() => noteBlockId("external"))}]);
    const updated = await query({search: "external update"}), incremental = copy(probe.metrics);
    check(updated.items[0]?.objectId === "reading-09" && !incremental.rows.documents,"external atomic incremental save");
    let conflicted = false;
    try {await other.commit(version,[{store: "documents",key: "page-0009",value: {damaged: true}}]);} catch {conflicted = true;}
    check(conflicted && (await query({search: "external update"})).items.length === 1,"stale CAS preserves projection");
    other.close?.(); other = undefined;
    db.close?.(); db = await openIndexedDatabase(name);
    probe.reset();
    check((await query({search: "external update"})).items.length === 1,"reopened persistent search");
    const reopened = copy(probe.metrics);
    check(!reopened.rows.documents && !reopened.writes.catalogBodies,"reopen cannot rebuild bodies");
    // Model an already deployed projection before object-order keys existed.
    const connection = await raw(name), tx = connection.transaction(["catalogHeaders","catalogState"],"readwrite"), complete = catalogTransactionDone(tx);
    const state = await catalogRequest(tx.objectStore("catalogState").get("current")) as BrowserCatalogState;
    tx.objectStore("catalogState").put({...state,searchMetadataReady: false},"current");
    await new Promise<void>((resolve,reject) => {
      const request = tx.objectStore("catalogHeaders").openCursor();
      request.onerror = () => reject(request.error);
      request.onsuccess = () => {const cursor = request.result;if (!cursor) {resolve();return;}
        const header = cursor.value;cursor.update({...header,orderKeys: header.orderKeys.filter((key: readonly [string,unknown,string]) => !key[0].startsWith("objects/"))});cursor.continue();};
    });
    await complete; connection.close();
    probe.failSecondBatch(); probe.reset();
    let failed = false;
    try {await query({limit: 20});} catch (error) {
      failed = error instanceof Error && "code" in error && error.code === "index_failed"
        && error.message.includes("note_catalog_search_prepare_failed") && error.message.includes("synthetic metadata second batch failure");
    }
    check(failed,"metadata batch failure cannot report ready");
    const inspect = await raw(name), progressTx = inspect.transaction("catalogState","readonly"), progressDone = catalogTransactionDone(progressTx);
    const progress = await catalogRequest(progressTx.objectStore("catalogState").get("current")) as BrowserCatalogState;
    await progressDone; inspect.close();
    check(progress.searchMetadataCursor === "page-0099" && !progress.searchMetadataReady,"only the first metadata batch commits");
    const retried = await query({limit: 20}), upgrade = copy(probe.metrics);
    check(retried.items.length === 20 && !upgrade.rows.documents && !upgrade.rows.catalogBodies,"metadata-only upgrade resumes without bodies");
    probe.reset();
    check((await query({limit: 20})).items.length === 20 && probe.metrics.rows.catalogHeaders === 21,"upgrade readiness persists");
    return {count: 2400,empty,preparation,rare,incremental,reopened,upgrade,mixedPages: [page1.items[0]?.objectId,page2.items[0]?.objectId,page3.items[0]?.objectId]};
  } finally {
    db?.close?.(); other?.close?.(); probe.restore();
    await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);request.onblocked = () => reject(Error("search probe cleanup blocked"));});
  }
}

/** Frozen 2000 ordinary + 2000 reading search budget, now measured against
 * durable IndexedDB rather than timing a mock record scan. Preparation is
 * explicitly measured separately from the ten ready-index queries. */
export async function verifyDenseNoteObjectSearch() {
  const name = "note-object-dense-" + crypto.randomUUID(), probe = instrument(name);
  let db: BrowserNotesDatabase | undefined;
  try {
    db = await openIndexedDatabase(name);
    const changes: NoteWrite[] = [];
    for (let i = 0; i < 4000; i++) {
      const id = notePageId("dense-" + String(i).padStart(4,"0")), timestamp = "2026-10-24T00:00:00Z";
      const page: NotePage = {id,title: "Synthetic " + i,position: i,status: "active",schemaVersion: 1,revision: 0,isFavorite: false,projectIds: [],tagIds: [],createdAt: timestamp,updatedAt: timestamp};
      changes.push({store: "pages",key: id,value: page},{store: "documents",key: id,value: createNoteDocumentFromText(id,i === 1999 || i === 3999 ? "needle match" : "Unrelated " + i,() => noteBlockId("dense-block-" + i))});
      if (i >= 2000) {
        const source: NoteReadingSource = {id: noteBlockId("reading-" + i),pageId: id,noteType: "highlight",color: "yellow",
          source: {paperId: "paper-" + i % 20,paperTitle: "Synthetic paper",attachmentId: "pdf-" + i % 20,contentSha256: "a".repeat(64),pageIndex: i % 30,rects: [],quoteExact: "quote",quotePrefix: "",quoteSuffix: "",state: "exact"}};
        changes.push({store: "sources",key: source.id,value: source});
      }
    }
    await db.commit(0,changes);
    console.info("[note-object-stage] 4000 canonical records seeded");
    check(db.searchNotes,"search port missing");
    const start = performance.now();
    await db.queryCatalog!({category: "all",view: "recent",search: "",limit: 20});
    const headersMs = performance.now() - start;
    console.info("[note-object-stage] 4000 headers prepared " + headersMs);
    check((await db.searchNotes({search: "needle",limit: 20})).items.length === 2,"dense initial matches");
    const preparationMs = performance.now() - start;
    console.info("[note-object-stage] 4000 search projection prepared " + preparationMs);
    probe.forbidBodies(); probe.reset();
    const samples: number[] = [];
    for (let i = 0; i < 10; i++) {
      const began = performance.now();
      const result = await db.searchNotes({search: "needle",limit: 20});
      samples.push(performance.now() - began);
      check(result.items.length === 2 && result.items.some(item => item.objectKind === "reading_note") && result.items.some(item => item.objectKind === "note_page"),"dense mixed matches");
    }
    samples.sort((left,right) => left - right);
    return {count: 4000,headersMs,preparationMs,p50: samples[5],p95: samples[9],metrics: copy(probe.metrics)};
  } finally {
    db?.close?.(); probe.restore();
    await new Promise<void>((resolve,reject) => {const request = indexedDB.deleteDatabase(name);request.onsuccess = () => resolve();request.onerror = () => reject(request.error);request.onblocked = () => reject(Error("dense cleanup blocked"));});
  }
}
