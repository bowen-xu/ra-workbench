import {decodeNoteSearchPosting,encodeNoteSearchPosting,NotesGatewayError} from "@research-assistant/data/notes";
import {catalogRequest} from "./browserCatalogRequests";

export const indexedPostingStore = "catalogPostingRows";
/** New rows own their count in a native key-only index. Legacy stores remain
 * untouched until a real membership change; upgrades create only an empty store. */
export function createIndexedPostingStore(database: IDBDatabase): void {
  const store = database.createObjectStore(indexedPostingStore,{keyPath: "gram"});
  store.createIndex("gramCount",["gram","count"],{unique: true});
}
function invalid(): never {
  throw new NotesGatewayError("index_failed","note_catalog_posting_row_invalid：搜索词项及计数记录损坏，请保留原始笔记并重建派生索引。");
}
function positiveCount(value: unknown): value is number {return Number.isSafeInteger(value) && (value as number) > 0;}
function decodeRow(raw: unknown, gram: string): Uint32Array {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return invalid();
  const row = raw as Record<string,unknown>;
  if (Object.keys(row).length !== 4 || Object.keys(row).some(key => !["schema","gram","count","value"].includes(key))
    || row.schema !== 1 || row.gram !== gram || !positiveCount(row.count)) return invalid();
  const value = decodeNoteSearchPosting(row.value);
  if (value.length !== row.count) return invalid();
  return value;
}
export async function readBrowserCatalogPosting(transaction: IDBTransaction, gram: string): Promise<{value: Uint32Array;legacy: boolean}> {
  if (transaction.objectStoreNames.contains(indexedPostingStore)) {
    const row = await catalogRequest(transaction.objectStore(indexedPostingStore).get(gram));
    if (row !== undefined) return {value: decodeRow(row,gram),legacy: false};
  }
  const old = await catalogRequest(transaction.objectStore("catalogPostings").get(gram));
  return {value: decodeNoteSearchPosting(old),legacy: old !== undefined};
}
export async function readBrowserCatalogPostingCount(transaction: IDBTransaction, gram: string): Promise<number | undefined> {
  if (transaction.objectStoreNames.contains(indexedPostingStore)) {
    const store = transaction.objectStore(indexedPostingStore);
    const cursor = await catalogRequest(store.index("gramCount").openKeyCursor(IDBKeyRange.bound([gram],[gram,[]])));
    if (cursor) {
      const key = cursor.key;
      if (!Array.isArray(key) || key.length !== 2 || key[0] !== gram || cursor.primaryKey !== gram || !positiveCount(key[1])) return invalid();
      return key[1];
    }
    // A malformed row can be absent from the secondary index. Do not turn
    // that corruption into a false zero count; this check reads only its key.
    if (await catalogRequest(store.getKey(gram)) !== undefined) return invalid();
  }
  const count = await catalogRequest(transaction.objectStore("catalogCounts").get(gram));
  if (count !== undefined && !positiveCount(count)) return invalid();
  if (count === undefined && await catalogRequest(transaction.objectStore("catalogPostings").getKey(gram)) !== undefined) return invalid();
  return count as number | undefined;
}
export function writeBrowserCatalogPosting(transaction: IDBTransaction, gram: string, value: Uint32Array, legacy: boolean): void {
  const old = transaction.objectStore("catalogPostings"), counts = transaction.objectStore("catalogCounts");
  if (!transaction.objectStoreNames.contains(indexedPostingStore)) {
    // Legacy projection transactions are retained for exact deployed-schema
    // fixtures. Production v7/v6 owners always include the new store.
    if (value.length) {old.put(encodeNoteSearchPosting(value),gram);counts.put(value.length,gram);}
    else {old.delete(gram);counts.delete(gram);}
    return;
  }
  const rows = transaction.objectStore(indexedPostingStore);
  if (value.length) rows.put({schema: 1,gram,count: value.length,value: encodeNoteSearchPosting(value)});
  else rows.delete(gram);
  if (legacy) {old.delete(gram);counts.delete(gram);}
}
