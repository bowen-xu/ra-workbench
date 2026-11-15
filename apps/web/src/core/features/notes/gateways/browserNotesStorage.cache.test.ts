/** @vitest-environment happy-dom */
import { afterEach, expect, it } from "vitest";
import { createNoteDocumentFromText, noteBlockId, notePageId } from "@research-assistant/core/notes";
import {selectAppStorage} from "../../../runtime/appStorage";
import {createTestNamespacedStorage} from "../../../runtime/testing/createTestNamespacedStorage";
import { readState, writeState } from "./browserNotesStorage";
import { configureBrowserNotesDatabase } from "./browserNotesDatabase";
import { createMemoryNotesDatabase } from "../../../runtime/testing/memoryNotesDatabase";
import {createIndexedPreviewStorage} from "../../../runtime/previewIndexedStorage";
import {flushStorage} from "../../../runtime/storagePersistence";

const key = "research-assistant.notes.v1";
afterEach(() => window.localStorage.clear());
function seed(count = 100) {
  localStorage.clear(); selectAppStorage(localStorage);
  const documents = Object.fromEntries(Array.from({length: count}, (_, index) => {
    const id = notePageId(`fixture-${index}`);
    return [id, createNoteDocumentFromText(id, "Synthetic text ".repeat(20), () => noteBlockId(`block-${index}`))];
  }));
  const raw = JSON.stringify({schemaVersion: 2, pages: [], documents, readingSources: [], links: [], revisions: []});
  localStorage.setItem(key, raw);
  return raw;
}
it("opens migrated preview records without a legacy snapshot or import marker", async () => {
  const memory = createMemoryNotesDatabase(); configureBrowserNotesDatabase(memory.open);
  const storage = await createIndexedPreviewStorage("cutover:"); selectAppStorage(storage);
  storage.setItem(key + ".indexeddb", "cutover"); storage.setItem(key, "retired corrupt snapshot");
  const document = createNoteDocumentFromText(notePageId("saved"), "已迁移正文", () => noteBlockId("block"));
  const db = await memory.open("research-assistant.notes.cutover");
  await db.commit(0, [{store: "documents", key: "saved", value: document}]);
  expect((await readState()).documents.saved).toEqual(document);
  expect(memory.metrics.commits).toBe(1);
  storage.removeItem(key); await flushStorage(storage);
  selectAppStorage(await createIndexedPreviewStorage("cutover:"));
  expect((await readState()).documents.saved).toEqual(document);
  expect((await db.read({navigation: ["import"]})).rows.navigation?.import).toBeUndefined();
});
it("starts a fresh preview without importing retired annotations or snapshots", async () => {
  const storage = await createIndexedPreviewStorage("fresh:"); selectAppStorage(storage);
  storage.setItem(key, "retired"); storage.setItem("research-assistant.annotations.v1", "retired");
  const state = await readState(); expect(state.pages).toEqual([]); expect(state.documents).toEqual({});
});
it("migrates the old snapshot once and reads only requested document records afterwards", async () => {
  const memory = createMemoryNotesDatabase(); configureBrowserNotesDatabase(memory.open);
  const raw = seed();
  await readState({metadataOnly: true});
  memory.metrics.documentReads = 0;
  for (let i = 0; i < 100; i++) await readState({metadataOnly: true});
  expect(memory.metrics.documentReads).toBe(0);
  expect(Object.keys((await readState({pageIds: ['fixture-1']})).documents)).toEqual(['fixture-1']);
  expect(memory.metrics.documentReads).toBe(1);
  expect(localStorage.getItem(key)).toBe(raw);
});
it("rejects stale writes and preserves bodies outside a partial metadata snapshot", async () => {
  configureBrowserNotesDatabase(createMemoryNotesDatabase().open); seed(2);
  const first = await readState({metadataOnly: true}), second = await readState({metadataOnly: true});
  await writeState({...first, favoriteOrder: []});
  await expect(writeState(second)).rejects.toMatchObject({code: 'revision_conflict'});
  expect(Object.keys((await readState()).documents)).toHaveLength(2);
});
it("does not publish a failed transaction and isolates a reset storage owner", async () => {
  const memory = createMemoryNotesDatabase(); let fail = false;
  configureBrowserNotesDatabase(async name => { const db = await memory.open(name); return {...db, commit: async (...args) => {if (fail) throw new Error('disk full'); await db.commit(...args);}}; });
  seed(1); const before = await readState(); fail = true;
  await expect(writeState({...before, documents: {}})).rejects.toThrow('disk full');
  expect(Object.keys((await readState()).documents)).toHaveLength(1);
  fail = false; localStorage.clear();
  expect(Object.keys((await readState()).documents)).toHaveLength(0);
});
it("keeps the original snapshot and refuses an incompletely verified migration", async () => {
  const memory = createMemoryNotesDatabase();
  configureBrowserNotesDatabase(async name => {
    const db = await memory.open(name);
    return {...db, commit: (version, changes) => db.commit(version, changes.filter(change => change.store !== "documents"))};
  });
  const raw = seed(2);
  await expect(readState()).rejects.toMatchObject({code: "storage_error", message: expect.stringContaining("notes_migration_verification_failed：笔记迁移校验失败（store=documents；expectedRows=2；actualRows=0；databaseVersion=1）")});
  expect(localStorage.getItem(key)).toBe(raw);
  await expect(readState()).rejects.toMatchObject({code: "revision_conflict", message: expect.stringContaining("databaseVersion=1；marker=pending")});
});
it("reports an absent migration marker separately without changing either copy", async () => {
  const memory = createMemoryNotesDatabase(); configureBrowserNotesDatabase(memory.open);
  const raw = seed(2); await readState();
  const db = await memory.open("research-assistant.notes." + localStorage.getItem(key + ".indexeddb"));
  const ready = await db.read({navigation: ["import"]});
  await db.commit(ready.version, [{store: "navigation", key: "import"}]);
  const before = await db.read({documents: "all", navigation: "all"});
  selectAppStorage(createTestNamespacedStorage(localStorage, ""));
  await expect(readState()).rejects.toMatchObject({code: "revision_conflict", message: expect.stringContaining(`databaseVersion=${before.version}；marker=missing`)});
  expect(await db.read({documents: "all", navigation: "all"})).toEqual(before);
  expect(localStorage.getItem(key)).toBe(raw);
});
it("coordinates two owners of the migrated database and detects legacy writes on reopen", async () => {
  configureBrowserNotesDatabase(createMemoryNotesDatabase().open);
  localStorage.clear();
  const first = createTestNamespacedStorage(localStorage, "isolated:"), second = createTestNamespacedStorage(localStorage, "isolated:");
  selectAppStorage(first); await readState();
  const before = await readState({metadataOnly: true});
  selectAppStorage(second); await readState();
  await writeState({...await readState({metadataOnly: true}), favoriteOrder: []});
  selectAppStorage(first);
  await expect(writeState(before)).rejects.toMatchObject({code: "revision_conflict"});
  second.setItem(key, JSON.stringify({schemaVersion: 2, pages: [], documents: {}, readingSources: [], links: [], revisions: []}));
  selectAppStorage(createTestNamespacedStorage(localStorage, "isolated:"));
  await expect(readState()).rejects.toMatchObject({code: "revision_conflict", message: expect.stringContaining("notes_legacy_snapshot_changed")});
});
it("ignores legacy storage events for other preview namespaces", async () => {
  configureBrowserNotesDatabase(createMemoryNotesDatabase().open);
  localStorage.clear(); selectAppStorage(createTestNamespacedStorage(localStorage, "current:"));
  await readState({metadataOnly: true});
  window.dispatchEvent(new StorageEvent("storage", {key: "unrelated:" + key}));
  await expect(readState({metadataOnly: true})).resolves.toMatchObject({pages: []});
  window.dispatchEvent(new StorageEvent("storage", {key: "current:" + key}));
  await expect(readState({metadataOnly: true})).rejects.toMatchObject({code: "revision_conflict"});
});
