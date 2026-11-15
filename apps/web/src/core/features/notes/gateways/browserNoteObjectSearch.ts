import type {NotePage, NoteReadingSource} from "@research-assistant/core/notes";
import {NotesGatewayError, NOTE_SEARCH_GRAM_VERSION, noteCatalogObjectOrderKey, type NoteCatalogHeader, type NoteSearchQuery, type NoteSearchPage, type NoteSearchResult} from "@research-assistant/data/notes";
import {catalogScopesReady,CatalogScopeNotReady} from "./browserNoteCatalogScopes";
import {ensureBrowserNoteCatalog} from "./browserNoteCatalogOwner";
import {loadBrowserNoteCatalogRuntime} from "./browserNoteCatalogRuntime";
import {browserCatalogStores, catalogRequest, catalogTransactionDone, searchCandidates, type BrowserCatalogBody, type BrowserCatalogState} from "./browserNoteCatalogStore";

export function validateNoteObjectSearch(query: NoteSearchQuery): {readonly after: readonly [string,string];readonly limit: number} {
  const invalid = (message: string): never => {throw new NotesGatewayError("invalid_input",message);};
  if (!query || typeof query !== "object" || Array.isArray(query)) invalid("搜索参数无效");
  if (query.objectKinds !== undefined && (!Array.isArray(query.objectKinds) || query.objectKinds.some(kind => !["note_page","reading_note"].includes(kind)))) invalid("搜索对象类型无效");
  if (query.status !== undefined && !["active","archived","trashed"].includes(query.status)) invalid("搜索状态无效");
  for (const value of [query.search,query.projectId,query.tagId,query.paperId,query.noteType,query.sourceState])
    if (value !== undefined && (typeof value !== "string" || value.length > 4096)) invalid("搜索参数无效");
  if (query.isFavorite !== undefined && typeof query.isFavorite !== "boolean" || query.limit !== undefined && !Number.isSafeInteger(query.limit)) invalid("搜索参数无效");
  let after: readonly [string,string] = ["",""];
  if (query.cursor !== undefined && query.cursor !== "") {
    if (typeof query.cursor !== "string" || new TextEncoder().encode(query.cursor).length > 8192) invalid("搜索游标无效");
    let value: unknown;
    try {value = JSON.parse(query.cursor);} catch {invalid("搜索游标无效");}
    if (!Array.isArray(value) || value.length !== 2 || value.some(part => typeof part !== "string")) invalid("搜索游标无效");
    after = value as [string,string];
  }
  return {after,limit: Math.min(Math.max(query.limit ?? 100,1),500)};
}

/** Existing object search over the same persistent projection as the main list.
 * Canonical metadata, readiness, search text and source receipts share one read
 * snapshot. No document/history/operation store participates after preparation. */
export async function queryBrowserNoteObjects(database: IDBDatabase, query: NoteSearchQuery): Promise<NoteSearchPage> {
  for (let attempt = 0;; attempt++) {
    try {return await queryBrowserNoteObjectSnapshot(database,query);}
    catch (reason) {if (!(reason instanceof CatalogScopeNotReady) || attempt === 2) throw reason;}
  }
}

async function queryBrowserNoteObjectSnapshot(database: IDBDatabase, query: NoteSearchQuery): Promise<NoteSearchPage> {
  const {after,limit} = validateNoteObjectSearch(query);
  if (query.objectKinds?.length === 0) return {items: [],indexState: "ready"};
  const searching = Boolean(query.search?.trim()), runtime = searching ? await loadBrowserNoteCatalogRuntime() : undefined;
  const terms = runtime?.terms(query.search ?? "") ?? [];
  try {
    await ensureBrowserNoteCatalog(database,"metadata");
    await ensureBrowserNoteCatalog(database,"searchMetadata");
    if (terms.length) await ensureBrowserNoteCatalog(database,"search",undefined,[query.status ?? "active"]);
  } catch (reason) {
    throw reason instanceof NotesGatewayError ? reason : new NotesGatewayError("index_failed",`note_catalog_search_prepare_failed：${reason instanceof Error ? `${reason.name}: ${reason.message}` : String(reason)}`);
  }
  const transaction = database.transaction([...browserCatalogStores,"pages","sources"],"readonly"), done = catalogTransactionDone(transaction);
  try {
    const state = await catalogRequest(transaction.objectStore("catalogState").get("current")) as BrowserCatalogState | undefined;
    if (terms.length && !catalogScopesReady(state,"search",[query.status ?? "active"])) throw new CatalogScopeNotReady();
    if (!state?.headersReady || !state.searchMetadataReady || terms.length && (state.gramVersion !== NOTE_SEARCH_GRAM_VERSION || state.normalizationVersion !== runtime?.normalizationVersion || state.collationVersion !== runtime?.collationVersion))
      throw new NotesGatewayError("index_failed","note_catalog_search_not_ready：笔记搜索索引正在准备，当前列表仍可使用。");
    const bucket = `objects/${query.status ?? "active"}`;
    const lower: IDBValidKey = after[0] || after[1] ? [bucket,new TextEncoder().encode(after[0]),after[1]] : [bucket];
    const candidates = terms.length ? await searchCandidates(transaction,terms) : undefined;
    const headers = transaction.objectStore("catalogHeaders"), items: NoteSearchResult[] = [];
    const visit = async (header: NoteCatalogHeader) => {
      const target = header.item.target;
      if (target.kind !== "page" && target.kind !== "reading") throw new NotesGatewayError("index_failed","note_catalog_object_target_invalid：笔记对象目录目标无效。");
      const objectKind = target.kind === "reading" ? "reading_note" : "note_page";
      if (query.objectKinds && !query.objectKinds.includes(objectKind)) return;
      const page = await catalogRequest(transaction.objectStore("pages").get(target.pageId)) as NotePage | undefined;
      if (!page) throw new NotesGatewayError("index_failed","note_catalog_page_missing：笔记目录对应页面缺失。");
      if (page.status !== (query.status ?? "active") || query.isFavorite !== undefined && page.isFavorite !== query.isFavorite
        || query.projectId !== undefined && !page.projectIds.includes(query.projectId) || query.tagId !== undefined && !page.tagIds.includes(query.tagId)) return;
      const source = target.kind === "reading" ? await catalogRequest(transaction.objectStore("sources").get(target.noteId)) as NoteReadingSource | undefined : undefined;
      if (target.kind === "reading" && (!source || source.pageId !== page.id)) throw new NotesGatewayError("index_failed","note_catalog_source_missing：阅读笔记目录对应来源缺失。");
      if (source && (query.paperId !== undefined && source.source.paperId !== query.paperId || query.noteType !== undefined && source.noteType !== query.noteType || query.sourceState !== undefined && source.source.state !== query.sourceState)) return;
      let snippet = header.item.title;
      if (terms.length) {
        const body = await catalogRequest(transaction.objectStore("catalogBodies").get(header.item.id)) as BrowserCatalogBody | undefined;
        if (!body) throw new NotesGatewayError("index_failed","note_catalog_body_projection_missing：笔记搜索投影缺失，请重建派生索引。");
        if (!terms.every(term => header.metadataText.includes(term) || body.foldedText.includes(term))) return;
        snippet = "";
        let count = 0;
        for (const character of body.text) {if (count++ === 240) break; snippet += character;}
      }
      items.push({objectId: source?.id ?? page.id,objectKind,title: source?.source.paperTitle ?? page.title,snippet,updatedAt: page.updatedAt,...(source ? {source: source.source} : {})});
    };
    if (candidates && candidates.size <= (limit + 1) * 4) {
      const selected = await Promise.all([...candidates].map(id => catalogRequest(headers.get(id)) as Promise<NoteCatalogHeader | undefined>));
      const ordered = selected.map(header => {
        if (!header) throw new NotesGatewayError("index_failed","note_catalog_header_missing：笔记目录头部缺失。");
        return {header,key: noteCatalogObjectOrderKey(header) as unknown as IDBValidKey};
      }).filter(row => `objects/${row.header.item.status}` === bucket && indexedDB.cmp(row.key,lower) > 0)
        .sort((left,right) => indexedDB.cmp(left.key,right.key));
      for (const row of ordered) {await visit(row.header); if (items.length > limit) break;}
    } else if (!candidates || candidates.size) await new Promise<void>((resolve,reject) => {
      const request = headers.index("order").openKeyCursor(IDBKeyRange.bound(lower,[bucket,[]],Boolean(after[0] || after[1]),true));
      request.onerror = () => reject(request.error);
      request.onsuccess = () => {
        const cursor = request.result;
        if (!cursor || items.length > limit) {resolve(); return;}
        if (candidates && !candidates.has(String(cursor.primaryKey))) {cursor.continue(); return;}
        const row = headers.get(cursor.primaryKey);
        row.onerror = () => reject(row.error);
        row.onsuccess = () => {
          if (!row.result) {reject(new NotesGatewayError("index_failed","note_catalog_header_missing：笔记目录头部缺失。")); return;}
          visit(row.result as NoteCatalogHeader).then(() => {if (items.length > limit) resolve(); else cursor.continue();}).catch(reject);
        };
      };
    });
    const more = items.length > limit;
    items.length = Math.min(items.length,limit);
    const last = items.at(-1);
    await done;
    return {items,indexState: "ready",...(more && last ? {nextCursor: JSON.stringify([last.objectId,last.objectKind])} : {})};
  } catch (reason) {
    try {transaction.abort();} catch { /* settled */ }
    await done.catch(() => undefined);
    throw reason instanceof NotesGatewayError ? reason : new NotesGatewayError("storage_error",`note_catalog_object_query_failed：${reason instanceof Error ? reason.message : String(reason)}`);
  }
}
