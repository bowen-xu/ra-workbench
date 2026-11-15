import {indexedPostingStore} from "../notes/gateways/browserCatalogPostingRows";
import {NOTE_CATALOG_UTF8_ORDER_VERSION, NOTE_SEARCH_GRAM_VERSION, NotesGatewayError, type NoteCatalogRuntime, type WritingNoteCatalogIssue} from "@research-assistant/data/notes";
import {catalogRequest, type BrowserCatalogState} from "../notes/gateways/browserNoteCatalogStore";

export const writingCatalogStores = ["catalogHeaders", "catalogBodies", "catalogPostings", "catalogCounts", "catalogState", "writingCatalogFiles",indexedPostingStore] as const;
export type WritingCatalogStatus = "active" | "archived";
export interface WritingCatalogWork {
  readonly manuscriptId: string; readonly path: string; readonly sha256: string;
  readonly status: WritingCatalogStatus;
  readonly state: "pending" | "ready" | "error";
  readonly id?: string;
  readonly cause?: string;
}
export interface WritingCatalogState extends BrowserCatalogState {
  readonly writing: {
    readonly version: "writing-note-text/v1";
    readonly orderVersion: typeof NOTE_CATALOG_UTF8_ORDER_VERSION;
    readonly counts: Readonly<Record<WritingCatalogStatus, Readonly<Record<WritingCatalogWork["state"], number>>>>;
    readonly issues: Readonly<Record<WritingCatalogStatus, readonly WritingNoteCatalogIssue[]>>;
  };
}
export function emptyWritingCatalogState(): WritingCatalogState {
  return {generation: 0, nextOrdinal: 1, collationVersion: "", normalizationVersion: "", gramVersion: NOTE_SEARCH_GRAM_VERSION,
    searchReady: true, headersReady: true, writing: {version: "writing-note-text/v1", orderVersion: NOTE_CATALOG_UTF8_ORDER_VERSION,
      counts: {active: {pending: 0, ready: 0, error: 0}, archived: {pending: 0, ready: 0, error: 0}}, issues: {active: [], archived: []}}};
}
export function useWritingCatalogRuntime(state: WritingCatalogState, runtime: NoteCatalogRuntime): WritingCatalogState {
  if (state.writing.version !== "writing-note-text/v1" || state.writing.orderVersion !== NOTE_CATALOG_UTF8_ORDER_VERSION || state.gramVersion !== NOTE_SEARCH_GRAM_VERSION
    || state.collationVersion && state.collationVersion !== runtime.collationVersion || state.normalizationVersion && state.normalizationVersion !== runtime.normalizationVersion)
    throw new NotesGatewayError("index_failed", "writing_note_catalog_projection_version：写作笔记查询投影版本不匹配，原文件未改变。");
  return {...state, collationVersion: runtime.collationVersion, normalizationVersion: runtime.normalizationVersion};
}
export function changeWritingCatalogCounts(state: WritingCatalogState, before: WritingCatalogWork | undefined, after: WritingCatalogWork | undefined): WritingCatalogState {
  const counts = {active: {...state.writing.counts.active}, archived: {...state.writing.counts.archived}};
  if (before) counts[before.status][before.state]--;
  if (after) counts[after.status][after.state]++;
  if (Object.values(counts).some(row => Object.values(row).some(value => !Number.isSafeInteger(value) || value < 0)))
    throw new NotesGatewayError("index_failed", "writing_note_catalog_count_invalid：写作笔记准备计数不一致，原文件已保留。");
  return {...state, writing: {...state.writing, counts}};
}
export async function refreshWritingCatalogIssues(tx: IDBTransaction, state: WritingCatalogState): Promise<WritingCatalogState> {
  const issues: Record<WritingCatalogStatus, WritingNoteCatalogIssue[]> = {active: [], archived: []};
  for (const status of ["active", "archived"] as const) if (state.writing.counts[status].error) {
    await new Promise<void>((resolve, reject) => {
      const request = tx.objectStore("writingCatalogFiles").index("statusState").openCursor(IDBKeyRange.only([status,"error"]));
      request.onerror = () => reject(request.error);
      request.onsuccess = () => {
        const entry = request.result;
        if (!entry) { resolve(); return; }
        const work = entry.value as WritingCatalogWork;
        if (!work.cause) { reject(new NotesGatewayError("index_failed", "writing_note_catalog_error_missing：写作笔记失败原因缺失。")); return; }
        issues[status].push({manuscriptId: work.manuscriptId, path: work.path, cause: work.cause});
        if (issues[status].length === 5) resolve(); else entry.continue();
      };
    });
  }
  return {...state, writing: {...state.writing, issues}};
}
export async function writingCatalogState(tx: IDBTransaction): Promise<WritingCatalogState> {
  const state = await catalogRequest(tx.objectStore("catalogState").get("current")) as WritingCatalogState | undefined;
  if (!state?.writing) throw new NotesGatewayError("index_failed", "writing_note_catalog_not_ready：写作笔记目录尚未准备，原文件已保留。");
  return state;
}
