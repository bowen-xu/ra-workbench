import type {NoteIndexItem, NoteIndexQuery, NotePage} from "@research-assistant/core/notes";
import {NotesGatewayError} from "@research-assistant/data/notes";
import {loadBrowserNoteCatalogRuntime} from "./browserNoteCatalogRuntime";
import {applyBrowserNoteCatalogChanges} from "./browserNoteCatalogWrites";
import {catalogRequest, browserCatalogStores, catalogTransactionDone, queryBrowserNoteCatalog, validateNoteCatalogQuery, type BrowserCatalogState} from "./browserNoteCatalogStore";
import {ensureBrowserCatalogBodies} from "./browserNoteCatalogBodyPreparation";
import {catalogQueryStatuses,catalogStatuses,CatalogScopeNotReady,type CatalogStatus} from "./browserNoteCatalogScopes";
import {joinBrowserNoteCatalogPreparation} from "./browserNoteCatalogPreparation";
import type {NoteWrite} from "./browserNotesDatabase";

const projectionStores = [...browserCatalogStores,"pages","documents","sources","links","navigation","version"];

async function readState(database: IDBDatabase): Promise<BrowserCatalogState | undefined> {
  const transaction = database.transaction("catalogState","readonly"), done = catalogTransactionDone(transaction);
  try {
    const state = await catalogRequest(transaction.objectStore("catalogState").get("current")) as BrowserCatalogState | undefined;
    await done;
    return state;
  } catch (reason) { await done.catch(() => undefined); throw reason; }
}

/** Initialization is a resumable derived-index operation, not an authoring
 * migration. Metadata never loads the canonical document store. Search backfill
 * starts only on demand, outside the list-readiness path. Each batch observes
 * canonical data and derived writes in one transaction. */
export function ensureBrowserNoteCatalog(database: IDBDatabase, mode: "metadata" | "searchMetadata" | "search" | "titles", signal?: AbortSignal, statuses: readonly CatalogStatus[] = catalogStatuses): Promise<void> {
  if (mode === "search" || mode === "titles") return ensureBrowserCatalogBodies(database,mode,statuses,signal);
  const extended = mode === "searchMetadata";
  const readyFor = (state: BrowserCatalogState | undefined) => extended ? state?.searchMetadataReady : state?.headersReady;
  return joinBrowserNoteCatalogPreparation(database,mode,async batchSignal => {
    batchSignal.throwIfAborted();
    const ready = await readState(database);
    batchSignal.throwIfAborted();
    if (readyFor(ready)) return;
    const runtime = await loadBrowserNoteCatalogRuntime();
    let favorites: {version: number;positions: ReadonlyMap<string,number>} | undefined;
    for (;;) {
      batchSignal.throwIfAborted();
      const transaction = database.transaction(projectionStores.filter(name => name !== "documents"),"readwrite"), done = catalogTransactionDone(transaction);
      try {
        const states = transaction.objectStore("catalogState");
        const state = await catalogRequest(states.get("current")) as BrowserCatalogState | undefined;
        if (readyFor(state)) {await done;return;}
        const after = extended ? state?.searchMetadataCursor : state?.headerCursor;
        const rows: {id: string;page: NotePage}[] = [];
        await new Promise<void>((resolve,reject) => {
          const request = transaction.objectStore("pages").openCursor(after ? IDBKeyRange.lowerBound(after,true) : undefined);
          request.onerror = () => reject(request.error);
          request.onsuccess = () => {
            const cursor = request.result;
            if (!cursor) {resolve();return;}
            rows.push({id: String(cursor.key),page: cursor.value as NotePage});
            if (rows.length === 100) resolve();else cursor.continue();
          };
        });
        const version = await catalogRequest(transaction.objectStore("version").get("current")) as number | undefined ?? 0;
        if (!favorites || favorites.version !== version) {
          const order = await catalogRequest(transaction.objectStore("navigation").get("favoriteOrder")) as string[] | undefined ?? [];
          favorites = {version,positions: new Map(order.map((id,index) => [id,index]))};
        }
        await applyBrowserNoteCatalogChanges(transaction,rows.map(row => ({store: "pages",key: row.id,value: row.page})),runtime,{favoritePositions: favorites.positions});
        const current = await catalogRequest(states.get("current")) as BrowserCatalogState;
        const complete = rows.length < 100;
        const progress = extended ? {searchMetadataReady: complete,...(rows.length ? {searchMetadataCursor: rows.at(-1)!.id} : {})}
          : {headersReady: complete,searchMetadataReady: complete,...(rows.length ? {headerCursor: rows.at(-1)!.id} : {})};
        states.put({...current,...progress},"current");
        await done;
        if (complete) return;
        await new Promise<void>(resolve => setTimeout(resolve,0));
      } catch (reason) {
        try {transaction.abort();} catch { /* completed */ }
        await done.catch(() => undefined);throw reason;
      }
    }
  },signal);
}

/** Legacy reading titles need only the returned reading documents once. Normal
 * page listing reads no bodies. Already indexed/opened bodies are reused. */
async function ensureReadingTitles(database: IDBDatabase, items: readonly NoteIndexItem[], expectedGeneration: number, signal?: AbortSignal): Promise<number | undefined> {
  signal?.throwIfAborted();
  const selected = items.filter(item => item.target.kind === "reading");
  if (!selected.length) return;
  const existing = database.transaction("catalogBodies","readonly"), existingDone = catalogTransactionDone(existing);
  let present: (IDBValidKey | undefined)[];
  try {
    present = await Promise.all(selected.map(item => catalogRequest(existing.objectStore("catalogBodies").getKey(item.id))));
    await existingDone;
  } catch (reason) { await existingDone.catch(() => undefined); throw reason; }
  signal?.throwIfAborted();
  if (present.every(key => key !== undefined)) return;
  const runtime = await loadBrowserNoteCatalogRuntime();
  signal?.throwIfAborted();
  const transaction = database.transaction(projectionStores,"readwrite"), done = catalogTransactionDone(transaction);
  try {
    const state = await catalogRequest(transaction.objectStore("catalogState").get("current")) as BrowserCatalogState;
    if (state.generation !== expectedGeneration) throw new NotesGatewayError("revision_conflict", "note_catalog_changed：笔记目录已更新，请重新查询。", state.generation);
    const changes: NoteWrite[] = [], missingBodies = new Set<string>();
    for (const item of selected) {
      if (await catalogRequest(transaction.objectStore("catalogBodies").getKey(item.id))) continue;
      if (item.target.kind !== "reading") continue;
      const document = await catalogRequest(transaction.objectStore("documents").get(item.target.pageId));
      if (!document) throw new NotesGatewayError("index_failed", `note_catalog_document_missing：阅读笔记正文缺失（pageId=${item.target.pageId}）。`);
      missingBodies.add(item.target.pageId);
      changes.push({store: "documents",key: item.target.pageId,value: document});
    }
    if (changes.length) await applyBrowserNoteCatalogChanges(transaction,changes,runtime,{knownMissingBodies: missingBodies});
    const after = await catalogRequest(transaction.objectStore("catalogState").get("current")) as BrowserCatalogState;
    await done;
    return changes.length ? after.generation : undefined;
  } catch (reason) {
    try { transaction.abort(); } catch { /* completed */ }
    await done.catch(() => undefined);
    throw reason;
  }
}

export async function queryOwnedBrowserNoteCatalog(database: IDBDatabase, query: NoteIndexQuery, options: {readonly signal?: AbortSignal} = {}) {
  const {signal} = options;
  signal?.throwIfAborted();
  validateNoteCatalogQuery(query);
  const needsSearch = query.search.trim() !== "", titleSort = query.sort?.field === "title";
  const runtime = needsSearch ? await loadBrowserNoteCatalogRuntime() : undefined;
  if (runtime) runtime.terms(query.search); // Fail before scheduling any backfill.
  await ensureBrowserNoteCatalog(database,"metadata",signal);
  const statuses = catalogQueryStatuses(query);
  let result: Awaited<ReturnType<typeof queryBrowserNoteCatalog>> | undefined;
  for (let attempt = 0; attempt < 3; attempt++) {
    if (needsSearch) {
      await ensureBrowserNoteCatalog(database,"searchMetadata",signal);
      await ensureBrowserNoteCatalog(database,"search",signal,statuses);
    } else if (titleSort) await ensureBrowserNoteCatalog(database,"titles",signal,statuses);
    signal?.throwIfAborted();
    try {result = await queryBrowserNoteCatalog(database,query,runtime);break;}
    catch (reason) {if (!(reason instanceof CatalogScopeNotReady) || attempt === 2) throw reason;}
  }
  if (!result) throw new CatalogScopeNotReady();
  signal?.throwIfAborted();
  const titleGeneration = needsSearch || titleSort ? undefined : await ensureReadingTitles(database,result.items,result.revision,signal);
  if (titleGeneration !== undefined) {
    // Only derived display titles changed in that checked transaction. Existing
    // date/source/favorite positions remain valid. An intervening canonical edit
    // still fails the normal generation check; it is never silently rebound.
    const cursor = query.cursor ? JSON.stringify({...JSON.parse(query.cursor),generation: titleGeneration}) : undefined;
    const {cursor: _old, ...scope} = query;
    result = await queryBrowserNoteCatalog(database,{...scope,...(cursor ? {cursor} : {})},runtime);
  }
  signal?.throwIfAborted();
  return result;
}
