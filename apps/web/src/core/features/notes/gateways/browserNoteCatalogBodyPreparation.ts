import type {NotePage} from "@research-assistant/core/notes";
import {NotesGatewayError,type NoteCatalogHeader} from "@research-assistant/data/notes";
import {browserCatalogStores,catalogRequest,catalogTransactionDone,type BrowserCatalogState} from "./browserNoteCatalogStore";
import {catalogScopesReady,catalogStatuses,type BodyPreparation,type CatalogStatus} from "./browserNoteCatalogScopes";
import {joinBrowserNoteCatalogPreparation} from "./browserNoteCatalogPreparation";
import {loadBrowserNoteCatalogRuntime} from "./browserNoteCatalogRuntime";
import {applyBrowserNoteCatalogChanges} from "./browserNoteCatalogWrites";
import type {NoteWrite} from "./browserNotesDatabase";

/** Scan only the requested lifecycle range, in stable identity order. Reading
 * titles use their own range, so sorting never hydrates ordinary documents.
 * Cursors/completion and body projections commit in the same atomic batch. */
export async function ensureBrowserCatalogBodies(database: IDBDatabase, mode: BodyPreparation, statuses: readonly CatalogStatus[], signal?: AbortSignal): Promise<void> {
  signal?.throwIfAborted();
  const check = database.transaction("catalogState","readonly"), checked = catalogTransactionDone(check);
  let ready: BrowserCatalogState | undefined;
  try {ready = await catalogRequest(check.objectStore("catalogState").get("current")) as BrowserCatalogState | undefined;await checked;}
  catch (reason) {await checked.catch(() => undefined);throw reason;}
  signal?.throwIfAborted();
  if (catalogScopesReady(ready,mode,statuses)) return;
  for (const status of statuses) await joinBrowserNoteCatalogPreparation(database,`${mode}:${status}`,async batchSignal => {
    const runtime = await loadBrowserNoteCatalogRuntime();
    for (;;) {
      batchSignal.throwIfAborted();
      const transaction = database.transaction([...browserCatalogStores,"pages","documents","sources","links","navigation","version"],"readwrite");
      const done = catalogTransactionDone(transaction);
      try {
        const states = transaction.objectStore("catalogState");
        const state = await catalogRequest(states.get("current")) as BrowserCatalogState;
        if (catalogScopesReady(state,mode,[status])) {await done;return;}
        const after = (mode === "titles" ? state?.readingTitleScopes : state?.bodyScopes)?.[status]?.after;
        const prefix = mode === "titles" ? ["reading",status] : [status];
        const rows: {id: string;cursor: string;page?: NotePage}[] = [];
        await new Promise<void>((resolve,reject) => {
          const index = mode === "titles" ? transaction.objectStore("catalogHeaders").index("readingStatus") : transaction.objectStore("pages").index("statusId");
          const request = index.openCursor(IDBKeyRange.bound(after ? [...prefix,after] : prefix,[...prefix,[]],Boolean(after),true));
          request.onerror = () => reject(request.error);
          request.onsuccess = () => {
            const cursor = request.result;
            if (!cursor) {resolve();return;}
            if (mode === "titles") {
              const header = cursor.value as NoteCatalogHeader;
              if (header.item.target.kind !== "reading" || header.item.status !== status) {reject(new NotesGatewayError("index_failed","note_catalog_reading_projection_invalid：阅读标题目录损坏。"));return;}
              rows.push({id: header.item.target.pageId,cursor: header.item.id});
            } else rows.push({id: String(cursor.primaryKey),cursor: String((cursor.key as string[])[1]),page: cursor.value as NotePage});
            if (rows.length === 8) resolve();else cursor.continue();
          };
        });
        const changes: NoteWrite[] = [], missingBodies = new Set<string>();
        for (const row of rows) {
          if (await catalogRequest(transaction.objectStore("catalogBodies").getKey(`page:${row.id}`))) continue;
          const document = await catalogRequest(transaction.objectStore("documents").get(row.id));
          if (!document) throw new NotesGatewayError("index_failed",`note_catalog_document_missing：笔记正文缺失（pageId=${row.id}），原始元数据已保留。`);
          if (row.page) changes.push({store: "pages",key: row.id,value: row.page});
          missingBodies.add(row.id);
          changes.push({store: "documents",key: row.id,value: document});
        }
        await applyBrowserNoteCatalogChanges(transaction,changes,runtime,{knownMissingBodies: missingBodies});
        const current = await catalogRequest(states.get("current")) as BrowserCatalogState;
        const complete = rows.length < 8;
        const field = mode === "titles" ? "readingTitleScopes" : "bodyScopes";
        const progress = {...current[field],[status]: {ready: complete,...(rows.length ? {after: rows.at(-1)!.cursor} : after ? {after} : {})}};
        const next = {...current,[field]: progress};
        // Preserve deployed all-status completion, but never infer it from a
        // single selected scope or from an unrelated empty range.
        states.put({...next,searchReady: catalogScopesReady(next,"search",catalogStatuses),readingTitlesReady: catalogScopesReady(next,"titles",catalogStatuses)},"current");
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
