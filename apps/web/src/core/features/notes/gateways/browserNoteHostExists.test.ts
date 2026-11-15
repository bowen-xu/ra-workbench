// @vitest-environment happy-dom
import { expect, it } from "vitest";
import { selectAppStorage } from "../../../runtime/appStorage";
import { createTestNamespacedStorage } from "../../../runtime/testing/createTestNamespacedStorage";
import { createMemoryNotesDatabase } from "../../../runtime/testing/memoryNotesDatabase";
import { configureBrowserNotesDatabase } from "./browserNotesDatabase";
import { browserNoteHostExists, readState } from "./browserNotesStorage";

it("uses exact metadata keys, distinguishes source block IDs from page IDs, and reads zero bodies/revisions", async () => {
  const storage = createTestNamespacedStorage(localStorage, `${crypto.randomUUID()}:`), memory = createMemoryNotesDatabase();
  selectAppStorage(storage); configureBrowserNotesDatabase(memory.open);
  await readState({metadataOnly: true});
  const token = storage.getItem("research-assistant.notes.v1.indexeddb")!;
  const db = await memory.open(`research-assistant.notes.${token}`), {version} = await db.read({});
  await db.commit(version, [
    {store: "pages", key: "ordinary", value: {id: "ordinary"}},
    {store: "pages", key: "reading-page", value: {id: "reading-page"}},
    {store: "sources", key: "source-block", value: {id: "source-block", pageId: "reading-page"}},
    {store: "documents", key: "ordinary", value: "must not decode this body"},
    {store: "documents", key: "reading-page", value: "must not decode this body"},
    {store: "revisions", key: "old", value: "must not decode this revision"},
  ]);
  memory.metrics.documentReads = 0; memory.metrics.revisionReads = 0;
  expect(await browserNoteHostExists("note", "ordinary")).toBe(true);
  expect(await browserNoteHostExists("note", "reading-page")).toBe(false);
  expect(await browserNoteHostExists("readingNote", "source-block")).toBe(true);
  expect(await browserNoteHostExists("readingNote", "reading-page")).toBe(false);
  expect(await browserNoteHostExists("readingNote", "ordinary")).toBe(false);
  expect(await browserNoteHostExists("note", "missing")).toBe(false);
  expect(memory.metrics.documentReads).toBe(0); expect(memory.metrics.revisionReads).toBe(0);
  await db.commit(version + 1, [{store: "sources", key: "source-block"}, {store: "pages", key: "ordinary"}]);
  expect(await browserNoteHostExists("readingNote", "source-block")).toBe(false);
  expect(await browserNoteHostExists("note", "ordinary")).toBe(false);
});
