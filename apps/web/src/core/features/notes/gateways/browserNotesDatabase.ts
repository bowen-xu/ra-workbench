import {createIndexedPostingStore} from "./browserCatalogPostingRows";
import { NotesGatewayError, type NoteCatalogRuntime, type NotePageCatalogPage, type NoteSearchQuery, type NoteSearchPage } from "@research-assistant/data/notes";
import type {NoteIndexQuery} from "@research-assistant/core/notes";
import {browserCatalogStores, createCatalogStores} from "./browserNoteCatalogStore";
import {applyBrowserNoteCatalogChanges} from "./browserNoteCatalogWrites";
import {loadBrowserNoteCatalogRuntime} from "./browserNoteCatalogRuntime";
import {queryOwnedBrowserNoteCatalog} from "./browserNoteCatalogOwner";
import {queryBrowserNoteObjects} from "./browserNoteObjectSearch";

export const noteStores = ["pages", "documents", "sources", "links", "revisions", "navigation", "conflicts", "operations"] as const;
export type NoteStore = typeof noteStores[number];
export type StoredRows = Partial<Record<NoteStore, Record<string, unknown>>>;
export interface NoteIndexedRead {readonly index:"pageId"|"sourcePageId";readonly keys:readonly string[];readonly primaryKeys?:readonly string[]}
export type NoteRead = Partial<Record<NoteStore, "all" | readonly string[] | NoteIndexedRead>>;
export interface NoteWrite {readonly store: NoteStore; readonly key: string; readonly value?: unknown}
export type NoteDocumentSelector = (metadata: Readonly<StoredRows>, version: number) => readonly string[];
export interface BrowserNotesDatabase {
  /** Select document keys synchronously after metadata requests finish, inside
   * the same readonly transaction. The selector must not mutate its input. */
  read(query: NoteRead, selectDocuments?: NoteDocumentSelector): Promise<{version: number; rows: StoredRows}>;
  commit(expectedVersion: number, changes: readonly NoteWrite[]): Promise<void>;
  queryCatalog?(query: NoteIndexQuery, options?: {readonly signal?: AbortSignal}): Promise<NotePageCatalogPage>;
  searchNotes?(query: NoteSearchQuery): Promise<NoteSearchPage>;
  close?(): void;
}
type Factory = (name: string) => Promise<BrowserNotesDatabase>;
let databaseFactory: Factory = openIndexedDatabase;
/** Explicit mock-data port; production never falls back to non-durable memory. */
export function configureBrowserNotesDatabase(factory: Factory): void { databaseFactory = factory; }
export function openNotesDatabase(name: string): Promise<BrowserNotesDatabase> { return databaseFactory(name); }

function cause(reason: unknown): Error {
  return reason instanceof NotesGatewayError ? reason
    : new NotesGatewayError("storage_error", reason instanceof Error ? reason.message : String(reason));
}
export async function openIndexedDatabase(name: string): Promise<BrowserNotesDatabase> {
  if (typeof indexedDB === "undefined") throw new NotesGatewayError("storage_error", "浏览器不支持 IndexedDB，笔记未写入；请使用受支持的浏览器。");
  const database = await new Promise<IDBDatabase>((resolve, reject) => {
    let blocked = false;
    // v7 adds empty indexed posting rows; legacy projections stay readable.
    // Close older writers before rows can leave the legacy posting/count stores.
    const request = indexedDB.open(name, 8);
    request.onupgradeneeded = event => {
      if (event.oldVersion < 7) createIndexedPostingStore(request.result);
      if (event.oldVersion < 1) for (const store of [...noteStores, "version"]) request.result.createObjectStore(store);
      if (event.oldVersion < 2) {
        request.transaction!.objectStore("sources").createIndex("pageId","pageId");
        request.transaction!.objectStore("links").createIndex("sourcePageId","sourcePageId");
        createCatalogStores(request.result);
      }
      if (event.oldVersion < 3) {
        const headers = request.transaction!.objectStore("catalogHeaders");
        if (!headers.indexNames.contains("reading")) headers.createIndex("reading",["item.target.kind","item.id"]);
      }
      if (event.oldVersion < 4) {
        // Versionchange closes older writers before they can replace new order
        // keys with the old projection format. No canonical records are read.
        const states = request.transaction!.objectStore("catalogState"), state = states.get("current");
        state.onsuccess = () => {
          if (!state.result) return;
          const {searchMetadataCursor: _old,...previous} = state.result as import("./browserNoteCatalogStore").BrowserCatalogState;
          states.put({...previous,searchMetadataReady: false},"current");
        };
      }
      // v8 adds metadata-only range indexes. Existing readiness/projections and
      // all earlier upgrade phases stay intact; no canonical body is loaded.
      if (event.oldVersion < 8) {
        request.transaction!.objectStore("pages").createIndex("statusId",["status","id"]);
        request.transaction!.objectStore("catalogHeaders").createIndex("readingStatus",["item.target.kind","item.status","item.id"]);
      }
    };
    request.onerror = () => reject(cause(request.error));
    request.onblocked = () => {blocked = true;reject(new NotesGatewayError("database_busy", "笔记数据库升级被其他标签页阻挡，请关闭旧标签页后重试。"));};
    request.onsuccess = () => {if (blocked) request.result.close();else resolve(request.result);};
  });
  database.onversionchange = () => database.close();
  let runtime: NoteCatalogRuntime | undefined;
  const needsRuntime = new Error("note_catalog_prepare_runtime");
  const commitChanges = (expectedVersion: number, changes: readonly NoteWrite[]): Promise<void> => new Promise((resolve,reject) => {
    const changesCatalog=changes.some(change=>["pages","documents","sources","links","navigation"].includes(change.store));
    // All canonical and derived reads/writes below share the original CAS.
    const names = [...new Set([...changes.map(change => change.store),"pages","sources","links","navigation",...browserCatalogStores,"version"])];
    const transaction = database.transaction(names,"readwrite");
    let error: Error | undefined;
    const versionStore = transaction.objectStore("version"), request = versionStore.get("current");
    request.onsuccess = () => {
      const version: number = request.result ?? 0;
      if (version !== expectedVersion) {
        error = new NotesGatewayError("revision_conflict", "浏览器笔记已被另一个操作更新，请保留输入后重试。", version);
        transaction.abort(); return;
      }
      const stateRequest = transaction.objectStore("catalogState").get("current");
      stateRequest.onsuccess = () => {
        if (stateRequest.result && changesCatalog && !runtime) { error = needsRuntime; transaction.abort(); return; }
        void (async () => {
          if (stateRequest.result && changesCatalog) await applyBrowserNoteCatalogChanges(transaction,changes,runtime!);
          for (const change of changes) {
            const store = transaction.objectStore(change.store);
            if (change.value === undefined) store.delete(change.key); else store.put(change.value,change.key);
          }
          versionStore.put(version + 1,"current");
        })().catch(reason => { error = cause(reason); try { transaction.abort(); } catch { /* aborted */ } });
      };
    };
    transaction.oncomplete = () => resolve();
    transaction.onabort = transaction.onerror = () => reject(error ?? cause(transaction.error ?? new Error("笔记事务提交失败，已有数据已保留")));
  });
  return {
    queryCatalog: (query,options) => queryOwnedBrowserNoteCatalog(database,query,options),
    searchNotes: query => queryBrowserNoteObjects(database,query),
    close: () => database.close(),
    read: (query, selectDocuments) => new Promise((resolve, reject) => {
      if (selectDocuments && query.documents !== undefined) {
        reject(new NotesGatewayError("invalid_input", "正文读取不能同时指定固定键与元数据筛选。")); return;
      }
      const names = Object.keys(query) as NoteStore[];
      const stores = [...new Set([...names, ...(selectDocuments ? ["documents"] : []), "version"])];
      const transaction = database.transaction(stores, "readonly");
      const rows: StoredRows = {};
      let version = 0;
      let selectionError: Error | undefined;
      let pending = names.length + 1;
      const completed = () => {
        if (--pending !== 0 || !selectDocuments) return;
        try {
          // Queue the selected reads in this success callback, while the
          // metadata transaction is active. No second transaction can mix versions.
          const keys = new Set(selectDocuments(rows, version));
          const documents: Record<string, unknown> = {}; rows.documents = documents;
          const store = transaction.objectStore("documents");
          for (const key of keys) {
            const request = store.get(key);
            request.onsuccess = () => { if (request.result !== undefined) documents[key] = request.result; };
          }
        } catch (reason) { selectionError = cause(reason); transaction.abort(); }
      };
      const versionRequest = transaction.objectStore("version").get("current");
      versionRequest.onsuccess = () => { version = versionRequest.result ?? 0; completed(); };
      for (const name of names) {
        const store = transaction.objectStore(name), keys = query[name]!;
        const values: Record<string, unknown> = {}; rows[name] = values;
        if (keys === "all") {
          const cursor = store.openCursor();
          cursor.onsuccess = () => { const item = cursor.result; if (item) { values[String(item.key)] = item.value; item.continue(); } else completed(); };
        } else if(!Array.isArray(keys)) {
          const indexed=keys as NoteIndexedRead;
          if(!(name==="sources"&&indexed.index==="pageId"||name==="links"&&indexed.index==="sourcePageId")){
            selectionError=new NotesGatewayError("invalid_input","note_point_index_invalid：笔记点查索引无效。");transaction.abort();return;
          }
          let remaining=indexed.keys.length+(indexed.primaryKeys?.length??0);
          const finish=()=>{if(--remaining===0)completed();};
          if(!remaining)completed();
          for(const key of indexed.keys){
            const request=store.index(indexed.index).getAll(key);
            request.onsuccess=()=>{for(const value of request.result as {id:string}[])values[value.id]=value;finish();};
          }
          for(const key of indexed.primaryKeys??[]){
            const request=store.get(key);request.onsuccess=()=>{if(request.result!==undefined)values[key]=request.result;finish();};
          }
        } else {
          let remaining = keys.length;
          if (remaining === 0) completed();
          for (const key of keys) {
            const request = store.get(key);
            request.onsuccess = () => { if (request.result !== undefined) values[key] = request.result; if (--remaining === 0) completed(); };
          }
        }
      }
      transaction.oncomplete = () => resolve({version, rows});
      transaction.onabort = transaction.onerror = () => reject(selectionError ?? cause(transaction.error ?? new Error("读取笔记事务失败")));
    }),
    commit: async (expectedVersion,changes) => {
      try { await commitChanges(expectedVersion,changes); }
      catch (reason) {
        if (reason !== needsRuntime) throw reason;
        // Abort before any canonical mutation, initialize outside IDB, then
        // retry the same original CAS. An intervening edit still conflicts.
        runtime = await loadBrowserNoteCatalogRuntime();
        await commitChanges(expectedVersion,changes);
      }
    },
  };
}
