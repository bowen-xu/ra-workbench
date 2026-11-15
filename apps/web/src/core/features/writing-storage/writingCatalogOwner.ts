import {decodeWritingNoteCatalogPage, decodeWritingNoteCatalogPreparation, NOTE_CATALOG_COLLATION_VERSION, NOTE_CATALOG_UTF8_ORDER_VERSION, NOTE_SEARCH_GRAM_VERSION, NotesGatewayError, type NoteCatalogRuntime, type WritingNoteCatalogGateway} from "@research-assistant/data/notes";
import {catalogRequest, catalogTransactionDone, queryBrowserNoteCatalogSnapshot, validateNoteCatalogQuery} from "../notes/gateways/browserNoteCatalogStore";
import {loadBrowserNoteCatalogRuntime} from "../notes/gateways/browserNoteCatalogRuntime";
import type {WritingRecordChange} from "./browserWritingRecords";
import {decodeWritingBody, decodeWritingFile} from "./writingRecordCodec";
import {projectWritingCatalogContent, type WritingCatalogContent} from "./writingCatalogProjection";
import {applyWritingCatalogChanges} from "./writingCatalogWrites";
import {changeWritingCatalogCounts, refreshWritingCatalogIssues, useWritingCatalogRuntime, writingCatalogState, writingCatalogStores, type WritingCatalogState, type WritingCatalogWork} from "./writingCatalogState";

const projectionStores = [...writingCatalogStores,"writingProjects","writingFiles","writingBodies"];
async function initializeRuntime(database: IDBDatabase, runtime: NoteCatalogRuntime): Promise<void> {
  const read = database.transaction("catalogState","readonly"), readDone = catalogTransactionDone(read);
  try {
    const state = await writingCatalogState(read), compatible = useWritingCatalogRuntime(state,runtime); await readDone;
    if (state.collationVersion === compatible.collationVersion && state.normalizationVersion === compatible.normalizationVersion) return;
  } catch (reason) { await readDone.catch(() => undefined); throw reason; }
  const tx = database.transaction("catalogState","readwrite"), done = catalogTransactionDone(tx);
  try {
    const before = await writingCatalogState(tx), after = useWritingCatalogRuntime(before,runtime);
    if (before.collationVersion !== after.collationVersion || before.normalizationVersion !== after.normalizationVersion)
      tx.objectStore("catalogState").put({...after,generation: after.generation + 1},"current");
    await done;
  } catch (reason) { try { tx.abort(); } catch { /* completed */ } await done.catch(() => undefined); throw reason; }
}

/** Derived preparation reads at most eight canonical bodies. Parse/hash work
 * occurs after the read transaction, with hash/state rechecks before publishing. */
async function prepareBatch(database: IDBDatabase, retryErrors: boolean, scope: "active" | "archived" | "live") {
  const statuses: readonly ("active" | "archived")[] = scope === "live" ? ["active","archived"] : [scope];
  const runtime = await loadBrowserNoteCatalogRuntime();
  await initializeRuntime(database,runtime);
  if (retryErrors) {
    const tx = database.transaction(["writingCatalogFiles","catalogState"],"readwrite"), done = catalogTransactionDone(tx);
    try {
      let state = await writingCatalogState(tx), changed = false;
      for (const status of statuses) if (state.writing.counts[status].error) await new Promise<void>((resolve,reject) => {
        const request = tx.objectStore("writingCatalogFiles").index("statusState").openCursor(IDBKeyRange.only([status,"error"]));
        request.onerror = () => reject(request.error);
        request.onsuccess = () => {
          const entry = request.result;
          if (!entry) { resolve(); return; }
          const before = entry.value as WritingCatalogWork, {cause: _cause,id: _id,...rest} = before;
          const after: WritingCatalogWork = {...rest,state: "pending"};
          state = changeWritingCatalogCounts(state,before,after); entry.update(after); changed = true; entry.continue();
        };
      });
      if (changed) { state = await refreshWritingCatalogIssues(tx,state); tx.objectStore("catalogState").put({...state,generation: state.generation + 1},"current"); }
      await done;
    } catch (reason) { try { tx.abort(); } catch { /* completed */ } await done.catch(() => undefined); throw reason; }
  }
  const read = database.transaction(["writingCatalogFiles","writingFiles","writingBodies"],"readonly"), readDone = catalogTransactionDone(read);
  const selected: {work: WritingCatalogWork;key: string;file?: unknown;body?: unknown}[] = [];
  try {
    for (const status of statuses) if (selected.length < 8) await new Promise<void>((resolve,reject) => {
      const request = read.objectStore("writingCatalogFiles").index("statusState").openCursor(IDBKeyRange.only([status,"pending"]));
      request.onerror = () => reject(request.error);
      request.onsuccess = () => {
        const entry = request.result;
        if (!entry) { resolve(); return; }
        selected.push({work: entry.value as WritingCatalogWork,key: String(entry.primaryKey)});
        if (selected.length === 8) resolve(); else entry.continue();
      };
    });
    for (const row of selected) {
      row.file = await catalogRequest(read.objectStore("writingFiles").get(row.key));
      row.body = await catalogRequest(read.objectStore("writingBodies").get(row.key));
    }
    await readDone;
  } catch (reason) { await readDone.catch(() => undefined); throw reason; }
  const prepared = new Map<string,WritingCatalogContent>();
  for (const row of selected) {
    if (!row.body || !row.file) { prepared.set(row.key,{error: "writing_note_document_missing：写作笔记正文或文件登记缺失；已有记录已保留。"}); continue; }
    try {
      const file = decodeWritingFile(row.file), body = decodeWritingBody(row.body);
      const hash = [...new Uint8Array(await crypto.subtle.digest("SHA-256",new TextEncoder().encode(body.content)))].map(byte => byte.toString(16).padStart(2,"0")).join("");
      prepared.set(row.key,file.sha256 === body.sha256 && file.sha256 === hash && file.relativePath === body.relativePath
        ? projectWritingCatalogContent(body.content) : {error: "writing_note_catalog_hash_mismatch：正文与文件登记不一致；原记录已保留。"});
    } catch { prepared.set(row.key,{error: "writing_note_catalog_record_invalid：写作笔记文件记录无效；原记录已保留。"}); }
  }
  const tx = database.transaction(projectionStores,"readwrite"), done = catalogTransactionDone(tx);
  try {
    const changes: WritingRecordChange[] = [];
    for (const row of selected) {
      const current = await catalogRequest(tx.objectStore("writingCatalogFiles").get(row.key)) as WritingCatalogWork | undefined;
      const file = await catalogRequest(tx.objectStore("writingFiles").get(row.key));
      if (current?.state !== "pending" || current.sha256 !== row.work.sha256 || !file || decodeWritingFile(file).sha256 !== row.work.sha256) continue;
      changes.push({store: "writingFiles",key: row.key,value: file});
    }
    if (changes.length) await applyWritingCatalogChanges(tx,changes,runtime,prepared);
    const state = await writingCatalogState(tx);
    await done;
    return decodeWritingNoteCatalogPreparation({processedCount: changes.length,
      pendingCount: statuses.reduce((sum,status) => sum + state.writing.counts[status].pending,0),
      errorCount: statuses.reduce((sum,status) => sum + state.writing.counts[status].error,0)});
  } catch (reason) { try { tx.abort(); } catch { /* completed */ } await done.catch(() => undefined); throw reason; }
}

export function createWritingCatalogOwner(database: IDBDatabase): WritingNoteCatalogGateway {
  let pending: {retry: boolean; scope: "active" | "archived" | "live"; promise: ReturnType<typeof prepareBatch>} | undefined;
  return {
    async queryWritingNoteCatalog(input) {
      if (!input) throw new NotesGatewayError("invalid_input","note_catalog_query_invalid：笔记查询参数无效。");
      const query = {...input,search: input.search === undefined ? "" : input.search};
      const limit = validateNoteCatalogQuery(query), searching = query.search.trim() !== "";
      const runtime = searching ? await loadBrowserNoteCatalogRuntime() : undefined;
      if (runtime) { runtime.terms(query.search); await initializeRuntime(database,runtime); }
      const result = await queryBrowserNoteCatalogSnapshot(database,query,runtime), state = result.state as WritingCatalogState;
      if (!state.writing || state.writing.version !== "writing-note-text/v1" || state.writing.orderVersion !== NOTE_CATALOG_UTF8_ORDER_VERSION || state.gramVersion !== NOTE_SEARCH_GRAM_VERSION
        || state.collationVersion && state.collationVersion !== NOTE_CATALOG_COLLATION_VERSION)
        throw new NotesGatewayError("index_failed","writing_note_catalog_not_ready：写作笔记目录版本无效，原文件已保留。");
      const statuses = query.view === "trash" || !searching && (query.view === "favorites" || query.category === "independent") ? []
        : searching ? ["active","archived"] as const : [query.view === "archive" ? "archived" : "active"] as const;
      const pendingCount = statuses.reduce((count,status) => count + state.writing.counts[status].pending,0);
      const errorCount = statuses.reduce((count,status) => count + state.writing.counts[status].error,0);
      return decodeWritingNoteCatalogPage({...result.page,pendingCount,errorCount,issues: statuses.flatMap(status => state.writing.issues[status]).slice(0,5)},limit);
    },
    prepareWritingNoteCatalog(options = {}) {
      if (!options || typeof options !== "object" || Array.isArray(options) || Object.keys(options).some(key => !["retryErrors","scope"].includes(key)) || options.retryErrors !== undefined && typeof options.retryErrors !== "boolean"
        || options.scope !== undefined && !["active","archived","live"].includes(options.scope))
        return Promise.reject(new NotesGatewayError("invalid_input","writing_note_catalog_preparation_invalid：写作笔记准备参数无效。"));
      const retry = options.retryErrors === true, scope = options.scope ?? "live";
      if (pending?.retry === retry && pending.scope === scope) return pending.promise;
      const previous = pending?.promise;
      const promise = previous ? previous.catch(() => undefined).then(() => prepareBatch(database,retry,scope)) : prepareBatch(database,retry,scope);
      const current = {retry,scope,promise}; pending = current;
      const clear = () => { if (pending === current) pending = undefined; };
      void promise.then(clear,clear); return promise;
    },
  };
}
