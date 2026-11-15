/** @vitest-environment happy-dom */
import { afterEach, beforeEach, expect, it } from "vitest";
import { createNoteDocumentFromText, noteBlockId, notePageId, type NotePage, type NoteReadingSource } from "@research-assistant/core/notes";
import { browserNotesGateway as gateway } from "./browserNotesGateway";
import { configureBrowserNotesDatabase } from "./browserNotesDatabase";
import { readState } from "./browserNotesStorage";
import { createMemoryNotesDatabase } from "../../../runtime/testing/memoryNotesDatabase";
import { selectAppStorage } from "../../../runtime/appStorage";

let memory: ReturnType<typeof createMemoryNotesDatabase>;
beforeEach(async () => {
  localStorage.clear(); selectAppStorage(localStorage);
  memory = createMemoryNotesDatabase(); configureBrowserNotesDatabase(memory.open);
  const pages: NotePage[] = Array.from({length: 500}, (_, i) => ({id: notePageId(`page-${String(i).padStart(3, "0")}`), title: `Title ${i}`,
    status: i < 12 ? ["active", "archived", "trashed"][i % 3] as NotePage["status"] : "active", position: i,
    schemaVersion: 1, revision: 0, isFavorite: false, projectIds: i % 2 ? [] : ["project"], tagIds: i % 2 ? [] : ["tag"], createdAt: "2026-10-23", updatedAt: "2026-10-23"}));
  const documents = Object.fromEntries(pages.map((page, i) => [page.id, createNoteDocumentFromText(page.id,
    `body-${i} ${"synthetic ".repeat(100)}`, () => noteBlockId(`block-${i}`))]));
  const readingSources: NoteReadingSource[] = pages.slice(0, 12).map((page, i) => ({id: noteBlockId(`reading-${i}`), pageId: page.id,
    noteType: "highlight", color: "green", source: {anchorId: `anchor-${i}`, paperId: i % 2 ? "other" : "paper", paperTitle: "Synthetic paper",
      attachmentId: "pdf", contentSha256: "a".repeat(64), pageIndex: i, rects: [], quoteExact: `quote-${i}`, quotePrefix: "", quoteSuffix: "", state: "exact"}}));
  const revisions = pages.map((page, i) => ({id: `revision-${i}`, pageId: page.id, document: documents[page.id], createdAt: page.createdAt, reason: "manual", isPinned: false}));
  localStorage.setItem("research-assistant.notes.v1", JSON.stringify({schemaVersion: 2, pages, documents, readingSources, revisions, links: []}));
  await readState({metadataOnly: true});
  Object.assign(memory.metrics, {reads: 0, documentReads: 0, revisionReads: 0, commits: 0});
});
afterEach(() => localStorage.clear());

it("loads only reading bodies across the three lifecycle queries, without touching history", async () => {
  const counts = [];
  for (const status of ["active", "archived", "trashed"] as const) counts.push((await gateway.listReadingNotes({status})).length);
  expect(counts).toEqual([4, 4, 4]);
  expect(memory.metrics.documentReads).toBe(12);
  expect(memory.metrics.revisionReads).toBe(0);
  expect(memory.metrics.reads).toBe(3);
  expect(memory.metrics.commits).toBe(0);
});
it("keeps filters, lexical cursors and paging while reading only the selected bodies", async () => {
  const notes = await gateway.listReadingNotes({paperId: "paper", projectId: "project", tagId: "tag", noteType: "highlight", sourceState: "exact", cursor: "reading-0", limit: 1});
  expect(notes.map(note => note.id)).toEqual(["reading-6"]);
  expect(notes[0]?.body).toContain("body-6");
  expect(memory.metrics.documentReads).toBe(1);
  expect(await gateway.listReadingNotes({paperId: "missing"})).toEqual([]);
  expect(memory.metrics.documentReads).toBe(1);
});
it("searches all eligible bodies before applying the result limit", async () => {
  const notes = await gateway.listReadingNotes({search: "body-9", limit: 1});
  expect(notes.map(note => note.id)).toEqual(["reading-9"]);
  expect(memory.metrics.documentReads).toBe(4);
  expect(memory.metrics.revisionReads).toBe(0);
});
it("resolves one reading note by either identity without loading unrelated bodies", async () => {
  const first = await gateway.getReadingNote(noteBlockId("reading-6"));
  const second = await gateway.getReadingNote(noteBlockId("page-006"));
  expect(first).toEqual(second);
  expect(first.pageId).toBe("page-006");
  expect(memory.metrics.documentReads).toBe(2);
  await expect(gateway.getReadingNote(noteBlockId("missing"))).rejects.toMatchObject({code: "not_found"});
  expect(memory.metrics.documentReads).toBe(2);
  expect(memory.metrics.revisionReads).toBe(0);
});
it("does not load ordinary note bodies when a paper has only dependent highlights", async () => {
  expect(await gateway.listLiteratureNotes("paper")).toEqual([]);
  expect(memory.metrics.documentReads).toBeLessThanOrEqual(2);
  expect(memory.metrics.revisionReads).toBe(0);
});
it("keeps pagination correct when an earlier eligible metadata record has lost its body", async () => {
  const database = await memory.open("research-assistant.notes." + localStorage.getItem("research-assistant.notes.v1.indexeddb"));
  const before = await database.read({});
  await database.commit(before.version, [{store: "documents", key: "page-000"}]);
  expect((await gateway.listReadingNotes({limit: 1})).map(note => note.id)).toEqual(["reading-3"]);
});
it("does not decode a damaged unrelated document, and still rejects a damaged selected body", async () => {
  const database = await memory.open("research-assistant.notes." + localStorage.getItem("research-assistant.notes.v1.indexeddb"));
  const before = await database.read({});
  await database.commit(before.version, [{store: "documents", key: "page-499", value: {damaged: true}}]);
  expect((await gateway.getReadingNote(noteBlockId("reading-6"))).body).toContain("body-6");
  await database.commit(before.version + 1, [{store: "documents", key: "page-006", value: {damaged: true}}]);
  await expect(gateway.getReadingNote(noteBlockId("reading-6"))).rejects.toThrow();
});
