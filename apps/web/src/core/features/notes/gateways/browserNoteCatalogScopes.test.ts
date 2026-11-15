import {describe,it,expect} from "vitest";
import {catalogQueryStatuses,catalogScopesReady,invalidateCatalogScope} from "./browserNoteCatalogScopes";
import type {BrowserCatalogState} from "./browserNoteCatalogStore";

const state: BrowserCatalogState = {generation: 4,nextOrdinal: 9,collationVersion: "test",normalizationVersion: "test",gramVersion: "test",searchReady: false};
describe("persistent lifecycle readiness",() => {
  it("matches normal/search lifecycle semantics independently of category",() => {
    expect(catalogQueryStatuses({view: "recent",search: ""})).toEqual(["active"]);
    expect(catalogQueryStatuses({view: "archive",search: ""})).toEqual(["archived"]);
    expect(catalogQueryStatuses({view: "archive",search: "needle"})).toEqual(["active","archived"]);
    expect(catalogQueryStatuses({view: "trash",search: "needle"})).toEqual(["trashed"]);
  });
  it("a body scope supplies titles but a title scope never supplies full search",() => {
    const progress = {...state,bodyScopes: {active: {ready: true}},readingTitleScopes: {archived: {ready: true}}};
    expect(catalogScopesReady(progress,"titles",["active","archived"])).toBe(true);
    expect(catalogScopesReady(progress,"search",["active","archived"])).toBe(false);
    expect(catalogScopesReady(progress,"search",["trashed"])).toBe(false);
  });
  it("deployed global completion survives invalidation only in unaffected scopes",() => {
    const previous = {...state,searchReady: true,readingTitlesReady: true,bodyCursor: "old",readingTitleCursor: "old",
      bodyScopes: {archived: {ready: false,after: "stale"}}};
    const invalid = invalidateCatalogScope(previous,"active");
    expect(catalogScopesReady(invalid,"search",["archived","trashed"])).toBe(true);
    expect(catalogScopesReady(invalid,"titles",["active"])).toBe(false);
    expect(invalid.bodyScopes?.active).toEqual({ready: false});
    expect(invalid.bodyCursor).toBeUndefined();
    expect(previous.searchReady).toBe(true);
    expect(invalid.generation).toBe(previous.generation);
  });
  it("restarts only the moved scope's cursor without changing other progress",() => {
    const progress = {...state,bodyScopes: {active: {ready: false,after: "a"},archived: {ready: false,after: "b"}}};
    const invalid = invalidateCatalogScope(progress,"active");
    expect(invalid.bodyScopes?.active).toEqual({ready: false});
    expect(invalid.bodyScopes?.archived).toEqual(progress.bodyScopes.archived);
  });
});
