/** @vitest-environment happy-dom */
import {afterEach, expect, it, vi} from "vitest";
import {selectAppStorage} from "../../../runtime/appStorage";
import {createMemoryNotesDatabase} from "../../../runtime/testing/memoryNotesDatabase";
import {configureBrowserNotesDatabase} from "./browserNotesDatabase";
import {browserNotesGateway} from "./browserNotesGateway";

const key = "research-assistant.notes.v1";
afterEach(() => {localStorage.clear();selectAppStorage(localStorage);});
function storage(token: string) {
  localStorage.clear();selectAppStorage(localStorage);localStorage.setItem(`${key}.indexeddb`,token);
}

it("routes the catalog through the same selected storage owner and validates metadata responses", async () => {
  storage("catalog-routes");
  const memory = createMemoryNotesDatabase(), queryCatalog = vi.fn().mockResolvedValue({revision: 2,items: []});
  configureBrowserNotesDatabase(async name => ({...await memory.open(name),queryCatalog}));
  const query = {category: "all" as const,view: "recent" as const,sort: {field: "source" as const,direction: "descending" as const},limit: 20};
  await expect(browserNotesGateway.pageCatalog!.queryNotePageCatalog(query)).resolves.toEqual({revision: 2,items: []});
  expect(queryCatalog).toHaveBeenCalledWith({...query,search: ""},undefined);
  expect(memory.metrics.documentReads).toBe(0);
  queryCatalog.mockResolvedValue({revision: 2,items: [],document: {private: true}});
  await expect(browserNotesGateway.pageCatalog!.queryNotePageCatalog(query)).rejects.toThrow("note_catalog_response_invalid");
});

it("retains the migration guard and never calls a catalog from incomplete canonical data", async () => {
  storage("catalog-blocked");
  const memory = createMemoryNotesDatabase(), queryCatalog = vi.fn();
  const database = await memory.open("research-assistant.notes.catalog-blocked");
  await database.commit(0,[{store: "navigation",key: "favoriteOrder",value: []}]);
  configureBrowserNotesDatabase(async name => ({...await memory.open(name),queryCatalog}));
  await expect(browserNotesGateway.pageCatalog!.queryNotePageCatalog({category: "all",view: "recent"})).rejects.toMatchObject({code: "revision_conflict"});
  expect(queryCatalog).not.toHaveBeenCalled();
  expect(memory.metrics.documentReads).toBe(0);
});

it("does not substitute full-body traversal for a missing adapter capability", async () => {
  storage("catalog-missing");
  const memory = createMemoryNotesDatabase();configureBrowserNotesDatabase(memory.open);
  await expect(browserNotesGateway.pageCatalog!.queryNotePageCatalog({category: "all",view: "recent"})).rejects.toThrow("note_catalog_adapter_unavailable");
  expect(memory.metrics.documentReads).toBe(0);
});

it("forwards cancellation through the selected storage owner", async () => {
  storage("catalog-cancel");
  const memory = createMemoryNotesDatabase(), queryCatalog = vi.fn().mockResolvedValue({revision: 0,items: []});
  configureBrowserNotesDatabase(async name => ({...await memory.open(name),queryCatalog}));
  const controller = new AbortController(), options = {signal: controller.signal};
  await browserNotesGateway.pageCatalog!.queryNotePageCatalog({category: "all",view: "recent"},options);
  expect(queryCatalog).toHaveBeenLastCalledWith({category: "all",view: "recent",search: ""},options);
  controller.abort();queryCatalog.mockClear();
  await expect(browserNotesGateway.pageCatalog!.queryNotePageCatalog({category: "all",view: "recent"},options)).rejects.toMatchObject({name: "AbortError"});
  expect(queryCatalog).not.toHaveBeenCalled();
});
