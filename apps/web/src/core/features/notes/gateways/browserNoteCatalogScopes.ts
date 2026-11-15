import type {NoteIndexQuery, NotePage} from "@research-assistant/core/notes";
import {NotesGatewayError} from "@research-assistant/data/notes";
import type {BrowserCatalogState} from "./browserNoteCatalogStore";

export type CatalogStatus = NotePage["status"];
export type BodyPreparation = "search" | "titles";
export interface CatalogScopeProgress {readonly ready: boolean; readonly after?: string}
export const catalogStatuses: readonly CatalogStatus[] = ["active","archived","trashed"];

export function catalogQueryStatuses(query: Pick<NoteIndexQuery,"view" | "search">): readonly CatalogStatus[] {
  if (query.view === "trash") return ["trashed"];
  if (query.search.trim()) return ["active","archived"];
  return [query.view === "archive" ? "archived" : "active"];
}

export function catalogScopesReady(state: BrowserCatalogState | undefined, mode: BodyPreparation, statuses: readonly CatalogStatus[]): boolean {
  return Boolean(state && (state.searchReady || mode === "titles" && state.readingTitlesReady || statuses.every(status =>
    state.bodyScopes?.[status]?.ready || mode === "titles" && state.readingTitleScopes?.[status]?.ready)));
}

/** Expand a deployed all-status completion flag before invalidating one scope.
 * A new/moved unindexed page may precede the saved cursor, so restart that
 * scope. Existing body rows still skip canonical reads on the next pass. */
export function invalidateCatalogScope(state: BrowserCatalogState, status: CatalogStatus): BrowserCatalogState {
  const bodies = Object.fromEntries(catalogStatuses.map(key => [key,{...state.bodyScopes?.[key],ready: Boolean(state.searchReady || state.bodyScopes?.[key]?.ready)}]));
  const titles = Object.fromEntries(catalogStatuses.map(key => [key,{...state.readingTitleScopes?.[key],ready: Boolean(state.searchReady || state.readingTitlesReady || state.readingTitleScopes?.[key]?.ready)}]));
  const {bodyCursor: _body,readingTitleCursor: _title,...rest} = state;
  return {...rest,searchReady: false,readingTitlesReady: false,bodyScopes: {...bodies,[status]: {ready: false}},readingTitleScopes: {...titles,[status]: {ready: false}}};
}

/** Only this readiness race is retryable; corruption and storage failures keep
 * their original diagnostics. The final page always validates its own snapshot. */
export class CatalogScopeNotReady extends NotesGatewayError {
  constructor() {super("index_failed","note_catalog_search_not_ready：笔记搜索索引正在准备，当前列表仍可使用。");}
}
