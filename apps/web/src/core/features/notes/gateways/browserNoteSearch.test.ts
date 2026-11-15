/** @vitest-environment happy-dom */
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {searchBrowserNotes as search} from "./browserNoteSearch";
import {configureBrowserNotesDatabase, type NoteRead} from "./browserNotesDatabase";
import {readState} from "./browserNotesStorage";
import {createMemoryNotesDatabase} from "../../../runtime/testing/memoryNotesDatabase";
import {selectAppStorage} from "../../../runtime/appStorage";
import type {NoteSearchPage,NoteSearchQuery} from "@research-assistant/data/notes";

let memory: ReturnType<typeof createMemoryNotesDatabase>;
const queries: NoteRead[] = [];
const queryPort = vi.fn<(query: NoteSearchQuery) => Promise<NoteSearchPage>>();
beforeEach(async () => {
  localStorage.clear(); selectAppStorage(localStorage);
  memory = createMemoryNotesDatabase();
  queryPort.mockReset().mockResolvedValue({items: [],indexState: "ready"});
  configureBrowserNotesDatabase(async name => {
    const database = await memory.open(name);
    return {...database,searchNotes: queryPort,read: (query,selector) => {queries.push(query); return database.read(query,selector);}};
  });
  await readState({metadataOnly: true});
  queries.length = 0;
  Object.assign(memory.metrics,{reads: 0,commits: 0,documentReads: 0,revisionReads: 0});
});
afterEach(() => localStorage.clear());

it("delegates all filters and cursors to the durable owner's search port without a snapshot scan", async () => {
  const input: NoteSearchQuery = {objectKinds: ["reading_note"],paperId: "paper",projectId: "project",tagId: "tag",isFavorite: true,
    noteType: "highlight",sourceState: "exact",cursor: JSON.stringify(["previous","reading_note"]),search: "needle",limit: 1};
  const result: NoteSearchPage = {items: [],indexState: "ready",nextCursor: "opaque-port-result"};
  queryPort.mockResolvedValueOnce(result);
  expect(await search(input)).toBe(result);
  expect(queryPort).toHaveBeenCalledWith(input);
  expect(queries).toEqual([]);
  expect(memory.metrics).toMatchObject({reads: 0,commits: 0,documentReads: 0,revisionReads: 0});
});

it("does not cache results or bypass the next authoritative query", async () => {
  queryPort.mockResolvedValueOnce({items: [],indexState: "dirty"});
  expect((await search({search: "first"})).indexState).toBe("dirty");
  expect((await search({search: "second"})).indexState).toBe("ready");
  expect(queryPort.mock.calls.map(([query]) => query.search)).toEqual(["first","second"]);
});

it("rejects invalid inputs before any storage or search work", async () => {
  for (const cursor of ["bad","[]",'["id",1]',"a".repeat(8193)]) await expect(search({cursor})).rejects.toMatchObject({code: "invalid_input",message: "搜索游标无效"});
  for (const input of [{status: "wrong"},{objectKinds: ["wrong"]},{limit: Number.NaN},{search: "a".repeat(4097)},{isFavorite: "true"}])
    await expect(search(input as unknown as NoteSearchQuery)).rejects.toMatchObject({code: "invalid_input"});
  expect(queryPort).not.toHaveBeenCalled();
  expect(queries).toEqual([]);
});

it("returns an explicitly empty object scope without accessing storage", async () => {
  expect(await search({objectKinds: []})).toEqual({items: [],indexState: "ready"});
  expect(queryPort).not.toHaveBeenCalled();
  expect(queries).toEqual([]);
});

it("preserves the original structured search failure", async () => {
  const failure = Object.assign(new Error("note_catalog_body_projection_missing"),{code: "index_failed"});
  queryPort.mockRejectedValueOnce(failure);
  await expect(search({search: "needle"})).rejects.toBe(failure);
  expect(memory.metrics.documentReads).toBe(0);
});

it("does not fall back to loading all bodies when the adapter is unavailable", async () => {
  localStorage.clear();
  configureBrowserNotesDatabase(memory.open);
  await readState({metadataOnly: true});
  Object.assign(memory.metrics,{reads: 0,documentReads: 0});
  await expect(search({})).rejects.toMatchObject({code: "storage_error",message: expect.stringContaining("note_search_adapter_unavailable")});
  expect(memory.metrics.reads).toBe(0);
});

it("requires import verification and its ready marker before invoking the search port", async () => {
  localStorage.clear();
  const raw = JSON.stringify({schemaVersion: 2,pages: [],documents: {},readingSources: [],revisions: [],links: []});
  localStorage.setItem("research-assistant.notes.v1",raw);
  configureBrowserNotesDatabase(async name => {
    const database = await memory.open(name);
    return {...database,searchNotes: queryPort,commit: async (version,changes) => {
      if (changes.length === 1 && changes[0]?.store === "navigation") throw new Error("import_ready_write_failed");
      await database.commit(version,changes);
    }};
  });
  await expect(search({})).rejects.toThrow("import_ready_write_failed");
  expect(queryPort).not.toHaveBeenCalled();
  expect(localStorage.getItem("research-assistant.notes.v1")).toBe(raw);
  await expect(search({})).rejects.toMatchObject({code: "revision_conflict",message: expect.stringContaining("marker=pending")});
});
