// @vitest-environment happy-dom
import {beforeEach, expect, it, vi} from "vitest";
import {createNoteDocumentFromText, noteBlockId, notePageId, type NoteIndexItem} from "@research-assistant/core/notes";
import {NotesGatewayError} from "@research-assistant/data/notes";
import {NotesClient} from "../../../../../../../apps/web/src/core/features/notes/NotesClient";
import {browserNotesGateway as gateway} from "../../../../../../../apps/web/src/core/features/notes/gateways/browserNotesGateway";
import {configureBrowserNotesDatabase} from "../../../../../../../apps/web/src/core/features/notes/gateways/browserNotesDatabase";
import {readState, writeState} from "../../../../../../../apps/web/src/core/features/notes/gateways/browserNotesStorage";
import {createMemoryNotesDatabase} from "../../../../../../../apps/web/src/core/runtime/testing/memoryNotesDatabase";
import {selectAppStorage} from "../../../../../../../apps/web/src/core/runtime/appStorage";
import {applyNoteBulkAction} from "../../../../../../../../plugins/ra-note-manager/src/features/notes/workspace/noteFavoriteActions";
beforeEach(() => {localStorage.clear(); selectAppStorage(localStorage);});
async function fixture(count: number, textSize: number) {
  const pages = Array.from({length: count}, (_, i) => ({id: notePageId("perf-" + i), title: "Synthetic " + i,
    position: i, schemaVersion: 1 as const, status: "active" as const, isFavorite: false, revision: 0,
    createdAt: "2026-10-23", updatedAt: "2026-10-23", projectIds: [], tagIds: []}));
  await writeState({schemaVersion: 2, pages, documents: Object.fromEntries(pages.map(page => [page.id,
    createNoteDocumentFromText(page.id, "x".repeat(textSize), () => noteBlockId("body-" + page.id))])), readingSources: [], links: [], revisions: []});
  return pages.map((page): NoteIndexItem => ({id: page.id, title: page.title, snippet: "", updatedAt: page.updatedAt,
    category: "independent", status: page.status, isFavorite: false, target: {kind: "page", pageId: page.id}}));
}
it("commits 3/50/500 deletes once each with no body or revision reads/writes and one notification", async () => {
  for (const count of [3, 50, 500]) {
    localStorage.clear();
    const memory = createMemoryNotesDatabase(); configureBrowserNotesDatabase(memory.open);
    const items = await fixture(500, 1000);
    const before = {...memory.metrics}, client = new NotesClient(gateway), notify = vi.fn();
    client.subscribe(notify);
    const started = performance.now();
    const result = await applyNoteBulkAction(client, items.slice(0, count), "trash");
    const ms = Math.round(performance.now() - started);
    expect(result.succeededIds).toHaveLength(count);
    expect(memory.metrics.commits - before.commits).toBe(1);
    expect(memory.metrics.documentReads - before.documentReads).toBe(0);
    expect(memory.metrics.documentWrites - before.documentWrites).toBe(0);
    expect(memory.metrics.revisionReads - before.revisionReads).toBe(0);
    expect(notify).toHaveBeenCalledOnce();
    process.stdout.write(JSON.stringify({scenario: "metadata batch, memory persistence", count, library: 500, ms, commits: 1, bodyReads: 0, bodyWrites: 0}) + "\n");
  }
});
it("retries an unknown completion with the same operation identity without applying twice", async () => {
  configureBrowserNotesDatabase(createMemoryNotesDatabase().open);
  const items = await fixture(3, 100);
  let lost = true;
  const apply = vi.fn<typeof gateway.applyMetadataBatch>(async request => {
    const receipt = await gateway.applyMetadataBatch(request);
    if (lost) {lost = false; throw new NotesGatewayError("operation_unknown", "reply lost");}
    return receipt;
  });
  const client = new NotesClient({...gateway, applyMetadataBatch: apply}), notify = vi.fn(); client.subscribe(notify);
  await applyNoteBulkAction(client, items, "trash");
  expect(apply).toHaveBeenCalledTimes(2);
  expect(apply.mock.calls[0]![0]).toEqual(apply.mock.calls[1]![0]);
  expect((await readState({metadataOnly: true})).pages.every(page => page.revision === 1)).toBe(true);
  expect(notify).toHaveBeenCalledOnce();
});
it("rejects overlapping ancestor/child conflicts as a group and preserves independent links", async () => {
  configureBrowserNotesDatabase(createMemoryNotesDatabase().open);
  const items = await fixture(3, 100), state = await readState();
  await writeState({...state, pages: state.pages.map((page, i) => i === 1 ? {...page, parentPageId: state.pages[0]!.id} : page)});
  const receipt = await gateway.applyMetadataBatch({operationId: "group", action: "trash", items: state.pages.map((page, i) => ({id: page.id, expectedRevision: i === 1 ? 100 : 0}))});
  expect(receipt.succeededIds).toEqual([items[2]!.target.kind === "page" ? items[2]!.target.pageId : ""]);
  expect(receipt.failures).toHaveLength(2);
  expect((await readState({metadataOnly: true})).pages.filter(page => page.status === "active")).toHaveLength(2);
});
it("serializes independent writes without hiding genuine page revision conflicts", async () => {
  configureBrowserNotesDatabase(createMemoryNotesDatabase().open);
  const [first, second] = await Promise.all([gateway.createPage({title: "concurrent A"}), gateway.createPage({title: "concurrent B"})]);
  expect((await gateway.listPages()).map(page => page.id)).toEqual(expect.arrayContaining([first.page.id, second.page.id]));
  const save = (page: typeof first) => gateway.saveDocument({document: page.document, expectedRevision: page.page.revision, saveReason: "manual"});
  await Promise.all([save(first), save(second)]);
  const current = await gateway.loadDocument(first.page.id);
  const same = await Promise.allSettled([save(current), save(current)]);
  expect(same.map(result => result.status)).toEqual(["fulfilled", "rejected"]);
  expect(same[1]).toMatchObject({reason: {code: "revision_conflict"}});
});
it("does not touch unrelated large bodies or persisted history during a small delete", async () => {
  const memory = createMemoryNotesDatabase(); configureBrowserNotesDatabase(memory.open);
  const items = await fixture(200, 64000), beforeState = await readState();
  await writeState({...beforeState, revisions: [{id: "large-history" as import("@research-assistant/core/notes").NoteRevisionId,
    pageId: beforeState.pages[0]!.id, pageRevision: 0, reason: "manual", createdAt: "2026-10-23", isPinned: true,
    document: beforeState.documents[beforeState.pages[0]!.id]!}]});
  const before = {...memory.metrics};
  await applyNoteBulkAction(new NotesClient(gateway), items.slice(0, 3), "trash");
  expect(memory.metrics.documentReads - before.documentReads).toBe(0);
  expect(memory.metrics.documentWrites - before.documentWrites).toBe(0);
  expect(memory.metrics.revisionReads - before.revisionReads).toBe(0);
  expect(memory.metrics.commits - before.commits).toBe(1);
});
