import {catalogRequest,catalogTransactionDone} from "./browserCatalogRequests";
export {catalogRequest,catalogTransactionDone} from "./browserCatalogRequests";
import {indexedPostingStore,readBrowserCatalogPosting,readBrowserCatalogPostingCount} from "./browserCatalogPostingRows";
import {decodeNoteCatalogCursor, noteCatalogBucket, noteSearchCandidateOrdinals, intersectNoteSearchCandidates, noteSearchQueryGrams, NOTE_SEARCH_GRAM_VERSION, type NoteCatalogHeader, type NoteCatalogRuntime, type NoteCatalogCursor, NotesGatewayError} from "@research-assistant/data/notes";
import type {NoteIndexItem, NoteIndexQuery} from "@research-assistant/core/notes";
import {catalogQueryStatuses,catalogScopesReady,CatalogScopeNotReady,type CatalogStatus,type CatalogScopeProgress} from "./browserNoteCatalogScopes";

/** Frozen stores used by deployed pre-v7/v6 schema construction. */
export const catalogStores = ["catalogHeaders", "catalogBodies", "catalogPostings", "catalogCounts", "catalogState"] as const;
export const browserCatalogStores = [...catalogStores,indexedPostingStore] as const;
export interface BrowserCatalogState {
  readonly generation: number;
  readonly nextOrdinal: number;
  readonly collationVersion: string;
  readonly normalizationVersion: string;
  readonly gramVersion: string;
  /** Legacy derived backfill is separate from first-page readiness. */
  readonly searchReady: boolean;
  readonly headersReady?: boolean;
  readonly headerCursor?: string;
  readonly bodyCursor?: string;
  readonly readingTitlesReady?: boolean;
  readonly readingTitleCursor?: string;
  /** Object-ID order keys and the reading-paper title metadata projection. */
  readonly searchMetadataReady?: boolean;
  readonly searchMetadataCursor?: string;
  readonly bodyScopes?: Partial<Record<CatalogStatus,CatalogScopeProgress>>;
  readonly readingTitleScopes?: Partial<Record<CatalogStatus,CatalogScopeProgress>>;
}
export interface BrowserCatalogBody {readonly text: string; readonly foldedText: string; readonly grams: readonly string[]}

export function createCatalogStores(database: IDBDatabase): void {
  const headers = database.createObjectStore("catalogHeaders", {keyPath: "item.id"});
  headers.createIndex("order", "orderKeys", {multiEntry: true});
  headers.createIndex("ordinal", "ordinal", {unique: true});
  for (const name of catalogStores.slice(1)) database.createObjectStore(name);
}

export function validateNoteCatalogQuery(query: NoteIndexQuery): number {
  if (!query || Object.keys(query).some(key => !["category","view","search","sort","limit","cursor"].includes(key))
    || !["all","independent","dependent"].includes(query.category) || !["recent","favorites","archive","trash"].includes(query.view)
    || typeof query.search !== "string" || query.cursor !== undefined && typeof query.cursor !== "string"
    || query.sort && (Object.keys(query.sort).some(key => !["field","direction"].includes(key)) || !["title","category","source","updatedAt"].includes(query.sort.field) || !["ascending","descending"].includes(query.sort.direction)))
    throw new NotesGatewayError("invalid_input", "note_catalog_query_invalid：笔记查询参数无效。");
  const limit = query.limit ?? 20;
  if (!Number.isInteger(limit) || limit < 1 || limit > 200) throw new NotesGatewayError("invalid_input", "note_catalog_limit_invalid：笔记查询条数必须为 1 至 200。");
  return limit;
}

async function queryFingerprint(query: NoteIndexQuery, terms: readonly string[]): Promise<string> {
  try {
    const value = JSON.stringify([query.category,query.view,query.sort ?? null,terms]);
    return [...new Uint8Array(await crypto.subtle.digest("SHA-256",new TextEncoder().encode(value)))].map(byte => byte.toString(16).padStart(2,"0")).join("");
  } catch (reason) {
    throw new NotesGatewayError("index_failed",`note_catalog_scope_failed：查询游标无法生成。${reason instanceof Error ? `${reason.name}: ${reason.message}` : "hash_failed"}`);
  }
}

export async function searchCandidates(transaction: IDBTransaction, terms: readonly string[]): Promise<Set<string>> {
  const selected: Uint32Array[] = [];
  for (const term of terms) {
    const grams = noteSearchQueryGrams(term);
    const sizes = await Promise.all(grams.map(async gram => ({gram, count: await readBrowserCatalogPostingCount(transaction,gram)})));
    let smallest = sizes[0]!;
    for (const value of sizes) if ((value.count ?? 0) < (smallest.count ?? 0)) smallest = value;
    if (!smallest.count) return new Set();
    const {value: values} = await readBrowserCatalogPosting(transaction,smallest.gram);
    if (values.length !== smallest.count) throw new NotesGatewayError("index_failed", "note_catalog_postings_mismatch：笔记搜索索引记录不一致，请重建派生索引。");
    selected.push(noteSearchCandidateOrdinals(values));
  }
  const ordinals = intersectNoteSearchCandidates(selected), ids = new Set<string>();
  const headers = transaction.objectStore("catalogHeaders").index("ordinal");
  for (let start = 0; start < ordinals.length; start += 256) {
    const keys = await Promise.all([...ordinals.subarray(start,start + 256)].map(ordinal => catalogRequest(headers.getKey(ordinal))));
    for (const key of keys) {
      if (typeof key !== "string") throw new NotesGatewayError("index_failed", "note_catalog_ordinal_missing：笔记搜索索引身份缺失，请重建派生索引。");
      ids.add(key);
    }
  }
  return ids;
}

/** IndexedDB orders stored binary metadata keys. No canonical document store
 * participates in this transaction; bodies are saved search projections only. */
export async function queryBrowserNoteCatalog(database: IDBDatabase, query: NoteIndexQuery, runtime?: NoteCatalogRuntime): Promise<{
  readonly items: readonly NoteIndexItem[]; readonly revision: number; readonly nextCursor?: string;
}> {
  return (await queryBrowserNoteCatalogSnapshot(database,query,runtime)).page;
}

/** Source owners can return bounded readiness diagnostics from the exact same
 * snapshot as the page, rather than combining two independently read versions. */
export async function queryBrowserNoteCatalogSnapshot(database: IDBDatabase, query: NoteIndexQuery, runtime?: NoteCatalogRuntime): Promise<{
  readonly page: {readonly items: readonly NoteIndexItem[]; readonly revision: number; readonly nextCursor?: string};
  readonly state: BrowserCatalogState;
}> {
  const limit = validateNoteCatalogQuery(query), searching = query.search.trim() !== "";
  if (searching && !runtime) throw new NotesGatewayError("index_failed", "note_catalog_runtime_required：笔记搜索组件尚未准备完成。");
  const terms = searching ? runtime!.terms(query.search) : [];
  // Hash before opening the transaction. Escaped NUL/control characters can
  // make valid bounded terms exceed the cursor byte budget if embedded raw.
  const queryHash = searching ? await queryFingerprint(query,terms) : "";
  const transaction = database.transaction(searching ? [...browserCatalogStores] : ["catalogHeaders","catalogState"], "readonly");
  const done = catalogTransactionDone(transaction);
  try {
    const state = await catalogRequest(transaction.objectStore("catalogState").get("current")) as BrowserCatalogState | undefined;
    if (!state) throw new NotesGatewayError("index_failed", "note_catalog_not_ready：笔记目录正在准备，请稍后重试。");
    if (searching && !catalogScopesReady(state,"search",catalogQueryStatuses(query)) || !searching && query.sort?.field === "title" && !catalogScopesReady(state,"titles",catalogQueryStatuses(query)))
      throw new CatalogScopeNotReady();
    if (searching && (state.gramVersion !== NOTE_SEARCH_GRAM_VERSION || state.normalizationVersion !== runtime!.normalizationVersion || state.collationVersion !== runtime!.collationVersion))
      throw new NotesGatewayError("index_failed", "note_catalog_search_not_ready：笔记搜索索引正在准备，当前列表仍可使用。");
    const bucket = noteCatalogBucket(query, searching);
    const scope = {generation: state.generation,bucket,queryHash};
    const cursor = decodeNoteCatalogCursor(query.cursor, scope);
    const headers = transaction.objectStore("catalogHeaders");
    let after: IDBValidKey = [bucket];
    if (cursor) {
      const previous = await catalogRequest(headers.get(cursor.id)) as NoteCatalogHeader | undefined;
      const key = previous?.orderKeys.find(key => key[0] === bucket);
      if (!key) throw new NotesGatewayError("invalid_input", "note_catalog_cursor_missing：笔记目录游标对象不存在。");
      after = key as unknown as IDBValidKey;
    }
    const candidates = searching ? await searchCandidates(transaction, terms) : undefined;
    const items: NoteIndexItem[] = [];
    const searchItem = async (header: NoteCatalogHeader): Promise<NoteIndexItem | undefined> => {
      const body = await catalogRequest(transaction.objectStore("catalogBodies").get(header.item.id)) as BrowserCatalogBody | undefined;
      if (!body) throw new NotesGatewayError("index_failed", "note_catalog_body_projection_missing：笔记搜索投影缺失，请重建派生索引。");
      if (!terms.every(term => header.metadataText.includes(term) || body.foldedText.includes(term))) return;
      let snippet = "", count = 0;
      for (const character of body.text) { if (count++ === 240) break; snippet += character; }
      return {...header.item,snippet};
    };
    if (candidates && candidates.size <= limit * 4) {
      // A rare term should not walk every ordering key. Sort only this bounded
      // candidate plan (at most 800), using the database's own binary-key order.
      // Broad matches use the persistent order index below, never a full array
      // of metadata or bodies. The component/UI never receives this plan.
      const found = await Promise.all([...candidates].map(async id => {
        const header = await catalogRequest(headers.get(id)) as NoteCatalogHeader | undefined;
        if (!header) throw new NotesGatewayError("index_failed", "note_catalog_header_missing：笔记目录头部缺失。");
        const key = header.orderKeys.find(key => key[0] === bucket) as unknown as IDBValidKey | undefined;
        return key && (!cursor || indexedDB.cmp(key, after) > 0) ? {header,key} : undefined;
      }));
      const ordered = found.filter((value): value is NonNullable<typeof value> => value !== undefined)
        .sort((left,right) => indexedDB.cmp(left.key,right.key));
      for (const {header} of ordered) {
        const item = await searchItem(header);
        if (item) items.push(item);
        if (items.length > limit) break;
      }
    } else if (!candidates || candidates.size) await new Promise<void>((resolve, reject) => {
      // Array upper-bound cannot overlap a string bucket's next sibling.
      const range = IDBKeyRange.bound(after, [bucket, []], Boolean(cursor), true);
      const index = headers.index("order");
      const request = candidates ? index.openKeyCursor(range) : index.openCursor(range);
      request.onerror = () => reject(request.error);
      request.onsuccess = () => {
        const entry = request.result;
        if (!entry || items.length > limit) { resolve(); return; }
        if (candidates && !candidates.has(String(entry.primaryKey))) { entry.continue(); return; }
        if (!candidates && "value" in entry) {
          items.push((entry.value as NoteCatalogHeader).item); if (items.length > limit) resolve(); else entry.continue(); return;
        }
        const headerRequest = headers.get(entry.primaryKey);
        headerRequest.onerror = () => reject(headerRequest.error);
        headerRequest.onsuccess = () => {
          const header = headerRequest.result as NoteCatalogHeader | undefined;
          if (!header) { reject(new NotesGatewayError("index_failed", "note_catalog_header_missing：笔记目录头部缺失。")); return; }
          searchItem(header).then(item => {
            if (item) items.push(item);
            if (items.length > limit) resolve(); else entry.continue();
          }).catch(reject);
        };
      };
    });
    const hasMore = items.length > limit;
    items.length = Math.min(items.length, limit);
    const nextCursor: NoteCatalogCursor | undefined = hasMore ? {schema: 2,...scope,id: items.at(-1)!.id} : undefined;
    await done;
    return {page: {items, revision: state.generation,...(nextCursor ? {nextCursor: JSON.stringify(nextCursor)} : {})},state};
  } catch (reason) {
    // A synchronous validation failure can leave a live transaction. Settle its
    // receipt as well; never leak an unhandled rejection or close the database.
    try { transaction.abort(); } catch { /* already completed */ }
    await done.catch(() => undefined);
    throw reason instanceof NotesGatewayError ? reason : new NotesGatewayError("storage_error", `note_catalog_query_failed：${reason instanceof Error ? reason.message : String(reason)}`);
  }
}
