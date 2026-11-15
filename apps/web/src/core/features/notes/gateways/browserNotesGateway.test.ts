import {readState, writeState} from "./browserNotesStorage";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { noteBlockId, createNoteDocumentFromText, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { NotesGatewayError } from "@research-assistant/data/notes";

import { browserNotesGateway } from "./browserNotesGateway";

class MemoryStorage implements Storage {
  private readonly values = new Map<string, string>();
  get length(): number { return this.values.size; }
  clear(): void { this.values.clear(); }
  getItem(key: string): string | null { return this.values.get(key) ?? null; }
  key(index: number): string | null { return [...this.values.keys()][index] ?? null; }
  removeItem(key: string): void { this.values.delete(key); }
  setItem(key: string, value: string): void { this.values.set(key, value); }
}

const storage = new MemoryStorage();

beforeEach(() => {
  storage.clear();
  vi.stubGlobal("window", { localStorage: storage });
});

describe("browserNotesGateway", () => {
  it("creates item notes atomically without attachments and shares canonical document lifecycle", async () => {
    storage.setItem("research-assistant.literature.v1", JSON.stringify([{ id: "paper", title: "Item without PDF", attachments: [] }]));
    await expect(browserNotesGateway.createLiteratureNote("missing")).rejects.toMatchObject({ code: "not_found" });
    expect(await browserNotesGateway.listPages()).toHaveLength(0);
    const first = await browserNotesGateway.createLiteratureNote("paper");
    const second = await browserNotesGateway.createLiteratureNote("paper");
    const independent = await browserNotesGateway.createPage({ title: "文献笔记" });
    const summaries = await browserNotesGateway.listPages();
    expect(summaries.find(page => page.id === first.page.id)?.literaturePaperId).toBe("paper");
    expect(summaries.find(page => page.id === independent.page.id)?.literaturePaperId).toBeUndefined();
    expect((await browserNotesGateway.loadDocument(second.page.id)).page.literaturePaperId).toBe("paper");
    expect((await browserNotesGateway.listLiteratureNotes("paper")).map(note => note.page.id)).toEqual([second.page.id, first.page.id]);
    expect(await browserNotesGateway.listReadingNotes({ paperId: "paper" })).toEqual([]);
    const saved = await browserNotesGateway.saveDocument({ document: createNoteDocumentFromText(first.page.id, "Whole paper interpretation", () => noteBlockId(crypto.randomUUID())), expectedRevision: first.page.revision, saveReason: "manual" });
    expect((await browserNotesGateway.listLiteratureNotes("paper")).find(note => note.page.id === first.page.id)?.preview).toBe("Whole paper interpretation");
    expect(await browserNotesGateway.listLiteratureNotes("other-paper")).toEqual([]);
    const trashed = await browserNotesGateway.setPageStatus({ id: first.page.id, status: "trashed", expectedRevision: saved.page.revision });
    expect(await browserNotesGateway.listLiteratureNotes("paper")).toHaveLength(1);
    await browserNotesGateway.setPageStatus({ id: first.page.id, status: "active", expectedRevision: trashed.revision });
    expect(await browserNotesGateway.listLiteratureNotes("paper")).toHaveLength(2);
    expect((await browserNotesGateway.listBacklinks("paper", "paper")).every(link => link.sourceBlockId === undefined && link.relation === "derived_from")).toBe(true);
  });
  it("persists application documents and rejects stale revisions", async () => {
    const created = await browserNotesGateway.createPage({ title: "实验记录" });
    const document = {
      ...created.document,
      blocks: [{
        id: noteBlockId("block-1"),
        type: "paragraph" as const,
        props: {},
        content: { kind: "inline" as const, items: [{ type: "text" as const, text: "结果", styles: [] }] },
        children: [],
      }],
    };
    const saved = await browserNotesGateway.saveDocument({ document, expectedRevision: 0, saveReason: "manual" });
    expect(saved.page.revision).toBe(1);
    await expect(browserNotesGateway.saveDocument({ document, expectedRevision: 0, saveReason: "autosave" }))
      .rejects.toEqual(expect.objectContaining<Partial<NotesGatewayError>>({ code: "revision_conflict", currentRevision: 1 }));
    expect((await browserNotesGateway.loadDocument(created.page.id)).document.blocks[0]?.content)
      .toEqual(document.blocks[0]?.content);
  });

  it("migrates legacy PDF annotations once into reading note entities", async () => {
    storage.setItem("research-assistant.annotations.v1", JSON.stringify([{
      id: "annotation-1", anchorId: "anchor-1", itemId: "paper-1", attachmentId: "attachment-1",
      pageIndex: 0, rects: [], quoteExact: "source", annotationType: "note", color: "#fff",
      body: "comment", createdAt: "2026-08-02T00:00:00.000Z",
    }]));
    const notes = await browserNotesGateway.listReadingNotes();
    expect(notes).toHaveLength(1);
    expect(notes[0]).toEqual(expect.objectContaining({ id: "annotation-1", noteType: "annotation", body: "comment" }));
    expect(await browserNotesGateway.listReadingNotes()).toHaveLength(1);
  });

  it("shares one reading-note lifecycle with inline references and backlinks", async () => {
    storage.setItem("research-assistant.literature.v1", JSON.stringify([{
      id: "paper", title: "Source paper", attachments: [{ id: "pdf", contentSha256: "a".repeat(64) }],
    }]));
    const note = await browserNotesGateway.createReadingNote({
      noteType: "annotation", body: "initial", itemId: "paper", attachmentId: "pdf",
      pageIndex: 2, rects: [], quoteExact: "quoted\nresult",
      selector: {
        schemaVersion: 2,
        attachmentSha256: "a".repeat(64),
        segments: [
          { pageIndex: 2, rects: [{ x: 0.1, y: 0.1, width: 0.2, height: 0.03 }], exactText: "quoted" },
          { pageIndex: 3, rects: [{ x: 0.1, y: 0.1, width: 0.2, height: 0.03 }], exactText: "result" },
        ],
      },
    });
    expect(note.source.selector?.anchorId).toBe(note.source.anchorId);
    expect(note.source.selector?.segments).toHaveLength(2);
    const updated = await browserNotesGateway.saveDocument({ document: createNoteDocumentFromText(note.pageId!, "updated", () => noteBlockId(crypto.randomUUID())), expectedRevision: note.revision, saveReason: "manual" });
    const created = await browserNotesGateway.createPage({ title: "Synthesis" });
    const derived = await browserNotesGateway.saveDocument({ document: { ...created.document, blocks: [{
      id: noteBlockId("reference"), type: "paragraph", props: {}, children: [],
      content: { kind: "inline", items: [{ type: "mention", targetKind: "reading_note", targetId: note.id, labelSnapshot: "Source paper" }] },
    }] }, expectedRevision: created.page.revision, saveReason: "manual" });
    expect((await browserNotesGateway.listBacklinks("reading_note", note.id))[0]?.sourcePageId).toBe(derived.page.id);
    const trashed = await browserNotesGateway.setReadingNoteStatus({ id: note.id, status: "trashed", expectedRevision: updated.page.revision });
    expect(await browserNotesGateway.listReadingNotes()).toEqual([]);
    const restored = await browserNotesGateway.setReadingNoteStatus({ id: note.id, status: "active", expectedRevision: trashed.revision });
    expect(restored).toEqual(expect.objectContaining({ id: note.id, body: "updated", status: "active" }));
    const trashedAgain = await browserNotesGateway.setReadingNoteStatus({ id: note.id, status: "trashed", expectedRevision: restored.revision });
    await browserNotesGateway.permanentlyDeleteReadingNote({ id: note.id, expectedRevision: trashedAgain.revision });
    await expect(browserNotesGateway.getReadingNote(note.id)).rejects.toEqual(expect.objectContaining<Partial<NotesGatewayError>>({ code: "not_found" }));
    const brokenBacklinks = await browserNotesGateway.listBacklinks("reading_note", note.id);
    expect(brokenBacklinks).toHaveLength(1);
    expect(brokenBacklinks.every((link) => link.isBroken && link.target.id === note.id)).toBe(true);
  });

  it("reuses an explicitly selected source anchor", async () => {
    storage.setItem("research-assistant.literature.v1", JSON.stringify([{
      id: "paper", title: "Source paper", attachments: [{ id: "pdf", contentSha256: "a".repeat(64) }],
    }]));
    const note = await browserNotesGateway.createReadingNote({
      sourceAnchorId: "anchor-existing", noteType: "annotation", body: "manual note",
      itemId: "paper", attachmentId: "pdf", pageIndex: 1, rects: [], quoteExact: "source",
    });
    expect(note.source.anchorId).toBe("anchor-existing");
  });

  it("stores managed assets by content hash", async () => {
    const first = await browserNotesGateway.importNoteAsset({ displayName: "data.csv", mimeType: "text/csv", bytes: [1, 2, 3] });
    const second = await browserNotesGateway.importNoteAsset({ displayName: "copy.csv", mimeType: "text/csv", bytes: [1, 2, 3] });
    expect(first.id).toBe(second.id);
    expect([...await browserNotesGateway.readNoteAsset(first.id)]).toEqual([1, 2, 3]);
    await expect(browserNotesGateway.cancelNoteAssetImport()).resolves.toBe(false);
  });

  it("prunes browser history after 14 or 30 days while retaining pinned versions", async () => {
    const created = await browserNotesGateway.createPage({ title: "History" });
    const first = await browserNotesGateway.saveDocument({
      document: created.document,
      expectedRevision: 0,
      saveReason: "manual",
    });
    const second = await browserNotesGateway.saveDocument({
      document: first.document,
      expectedRevision: 1,
      saveReason: "autosave",
    });
    const loaded = await readState();
    // Persistence carries the live storage owner's identity; clone only the
    // history rows being edited, rather than serializing that capability.
    const state = {...loaded, revisions: loaded.revisions.map(revision => ({...revision}))};
    state.revisions[0] = {
      ...state.revisions[0]!,
      reason: "manual",
      createdAt: new Date(Date.now() - 31 * 24 * 60 * 60 * 1000).toISOString(),
      isPinned: false,
    };
    state.revisions[1] = {
      ...state.revisions[1]!,
      reason: "autosave",
      createdAt: new Date(Date.now() - 15 * 24 * 60 * 60 * 1000).toISOString(),
      isPinned: true,
    };
    const pinnedId = state.revisions[1]!.id;
    await writeState(state);

    await browserNotesGateway.saveDocument({
      document: second.document,
      expectedRevision: 2,
      saveReason: "manual",
    });
    const revisions = await browserNotesGateway.listRevisions(created.page.id);
    expect(revisions).toHaveLength(2);
    expect(revisions).toContainEqual(expect.objectContaining({ id: pinnedId, isPinned: true }));
    expect(revisions.some((revision) => revision.pageRevision === 0)).toBe(false);
  });

  it("reorders siblings and rejects moving a page under its descendant", async () => {
    const first = await browserNotesGateway.createPage({ title: "一" });
    const second = await browserNotesGateway.createPage({ title: "二" });
    const third = await browserNotesGateway.createPage({ title: "三" });
    await browserNotesGateway.movePage({ pageId: third.page.id, position: 0, expectedRevision: 0 });
    expect((await browserNotesGateway.listPages()).map((page) => [page.title, page.position])).toEqual([["三", 0], ["一", 1], ["二", 2]]);
    const child = await browserNotesGateway.createPage({ title: "子页面", parentPageId: first.page.id });
    const refreshedFirst = (await browserNotesGateway.listPages()).find((page) => page.id === first.page.id);
    if (refreshedFirst === undefined) throw new Error("missing first page");
    await expect(browserNotesGateway.movePage({ pageId: first.page.id, parentPageId: child.page.id, position: 0, expectedRevision: refreshedFirst.revision }))
      .rejects.toEqual(expect.objectContaining<Partial<NotesGatewayError>>({ code: "invalid_input" }));
    expect(second.page.id).not.toBe(first.page.id);
  });

  it("keeps child pages as stable blocks across rename, trash, and restore", async () => {
    const parent = await browserNotesGateway.createPage({ title: "父页面" });
    const child = await browserNotesGateway.createPage({ title: "子页面", parentPageId: parent.page.id });
    let parentSnapshot = await browserNotesGateway.loadDocument(parent.page.id);
    expect(parentSnapshot.document.blocks).toContainEqual(expect.objectContaining({
      type: "childPage",
      content: expect.objectContaining({ targetId: child.page.id, labelSnapshot: "子页面" }),
    }));
    const renamed = await browserNotesGateway.updatePage({
      pageId: child.page.id,
      title: "新标题",
      expectedRevision: child.page.revision,
    });
    parentSnapshot = await browserNotesGateway.loadDocument(parent.page.id);
    expect(parentSnapshot.document.blocks.find((block) => block.type === "childPage")?.content)
      .toEqual(expect.objectContaining({ targetId: child.page.id, labelSnapshot: "新标题" }));
    const trashed = await browserNotesGateway.setPageStatus({ id: child.page.id, status: "trashed", expectedRevision: renamed.revision });
    expect((await browserNotesGateway.loadDocument(parent.page.id)).document.blocks.some((block) => block.type === "childPage")).toBe(false);
    await browserNotesGateway.setPageStatus({ id: child.page.id, status: "active", expectedRevision: trashed.revision });
    parentSnapshot = await browserNotesGateway.loadDocument(parent.page.id);
    expect(parentSnapshot.document.blocks.some((block) => block.type === "childPage")).toBe(true);
    await browserNotesGateway.saveDocument({
      document: { ...parentSnapshot.document, blocks: parentSnapshot.document.blocks.filter((block) => block.type !== "childPage") },
      expectedRevision: parentSnapshot.page.revision,
      saveReason: "structure",
    });
    expect((await browserNotesGateway.loadDocument(child.page.id)).page.status).toBe("trashed");
  });

  it("creates a child page at the requested parent block position", async () => {
    const parent = await browserNotesGateway.createPage({ title: "父页面" });
    const trailing = await browserNotesGateway.createPage({ title: "末尾子页面", parentPageId: parent.page.id });
    const leading = await browserNotesGateway.createPage({
      title: "光标子页面",
      parentPageId: parent.page.id,
      parentBlockPosition: 0,
    });
    const loaded = await browserNotesGateway.loadDocument(parent.page.id);
    expect(loaded.document.blocks.map((block) => [
      block.type,
      block.content.kind === "reference" ? block.content.targetId : undefined,
    ])).toEqual([
      ["childPage", leading.page.id],
      ["paragraph", undefined],
      ["childPage", trailing.page.id],
    ]);
    expect((await browserNotesGateway.listPages())
      .filter((page) => page.parentPageId === parent.page.id)
      .map((page) => [page.id, page.position, page.revision]))
      .toEqual([[leading.page.id, 0, 0], [trailing.page.id, 1, 1]]);
    await expect(browserNotesGateway.createPage({
      title: "非法子页面",
      parentPageId: parent.page.id,
      parentBlockPosition: -1,
    })).rejects.toEqual(expect.objectContaining<Partial<NotesGatewayError>>({ code: "invalid_input" }));
  });

  it("permanently deletes only trashed targets and retains broken links", async () => {
    const source = await browserNotesGateway.createPage({ title: "来源" });
    const target = await browserNotesGateway.createPage({ title: "目标" });
    await browserNotesGateway.upsertLink({
      sourcePageId: source.page.id,
      target: { kind: "note_page", id: target.page.id, labelSnapshot: "目标快照" },
      relation: "reference",
    });
    await expect(browserNotesGateway.permanentlyDeletePage({ id: target.page.id, expectedRevision: 0 }))
      .rejects.toEqual(expect.objectContaining<Partial<NotesGatewayError>>({ code: "invalid_input" }));
    const trashed = await browserNotesGateway.setPageStatus({ id: target.page.id, status: "trashed", expectedRevision: 0 });
    await browserNotesGateway.permanentlyDeletePage({ id: target.page.id, expectedRevision: trashed.revision });
    await expect(browserNotesGateway.loadDocument(target.page.id)).rejects.toEqual(expect.objectContaining<Partial<NotesGatewayError>>({ code: "not_found" }));
    expect(await browserNotesGateway.listOutgoing(source.page.id)).toEqual([
      expect.objectContaining({ isBroken: true, target: expect.objectContaining({ labelSnapshot: "目标快照" }) }),
    ]);
  });

  // Search pagination/source filters run against real IndexedDB in
  // tests/ui/note-object-search.pw.ts; this mock owns only canonical records.
});

it("keeps rich reading documents, version history and fixed source metadata in the ordinary page store", async () => {
  storage.setItem("research-assistant.literature.v1", JSON.stringify([{id:"paper",title:"Source",attachments:[{id:"pdf",contentSha256:"a".repeat(64)}]}]));
  const note = await browserNotesGateway.createReadingNote({ noteType:"annotation",body:"",itemId:"paper",attachmentId:"pdf",pageIndex:3,rects:[],quoteExact:"Immutable source" });
  expect(await browserNotesGateway.listPages()).toEqual([]);
  expect((await browserNotesGateway.listPages({kind:"all"}))[0]?.id).toBe(note.pageId);
  const asset = await browserNotesGateway.importNoteAsset({ displayName: "data.csv", mimeType: "text/csv", bytes: [65,44,66] });
  const document: NoteDocumentV1 = { ...createNoteDocumentFromText(note.pageId!, "First\nSecond", () => noteBlockId(crypto.randomUUID())), blocks: [
    ...createNoteDocumentFromText(note.pageId!, "First\nSecond", () => noteBlockId(crypto.randomUUID())).blocks,
    { id: noteBlockId("list"), type: "bulletListItem", props: {}, content: { kind: "inline", items: [{type:"text",text:"List",styles:["bold"]}] }, children: [] },
    { id: noteBlockId("formula"), type: "formulaBlock", props: {}, content: { kind: "formula", source: "E=mc^2" }, children: [] },
    { id: noteBlockId("table"), type: "table", props: {}, content: { kind: "table", rows: [[{ kind: "inline", items: [{type:"text",text:"Cell",styles:[]}] }]] }, children: [] },
    { id: noteBlockId("asset"), type: "file", props: {}, content: { kind: "asset", assetId: asset.id, displayName: asset.displayName, mimeType: asset.mimeType, caption: "Data" }, children: [] },
  ] };
  const saved = await browserNotesGateway.saveDocument({ document, expectedRevision:0, saveReason:"manual" });
  const metadata = await browserNotesGateway.updateReadingNote({id:note.id,color:"#123456",expectedRevision:saved.page.revision});
  expect(metadata.body).toContain("First\nSecond"); expect(metadata.body).toContain("E=mc^2"); expect(metadata.body).toContain("Cell"); expect(metadata.source).toEqual(note.source);
  const revised = await browserNotesGateway.saveDocument({ document:createNoteDocumentFromText(note.pageId!,"Later",()=>noteBlockId(crypto.randomUUID())),expectedRevision:metadata.revision,saveReason:"manual" });
  expect((await browserNotesGateway.getReadingNote(noteBlockId(String(note.pageId)))).body).toBe("Later");
  const history = await browserNotesGateway.listRevisions(note.pageId!);
  const restored = await browserNotesGateway.restoreRevision(note.pageId!,history.find(entry=>entry.pageRevision===metadata.revision)!.id,revised.page.revision);
  expect(restored.document).toEqual({...document, title: "Source"});
  expect((await browserNotesGateway.getReadingNote(note.id)).source).toEqual(note.source);
  const state = JSON.parse(JSON.stringify(await readState()));
  expect(state.readingNotes).toBeUndefined(); expect(state.readingSources[0]).not.toHaveProperty("body");
});

it("migrates literal text, properties and old aliases without colliding with ordinary pages", async () => {
  const ordinary = await browserNotesGateway.createPage({ title: "Existing page" });
  const original = JSON.parse(JSON.stringify(await readState()));
  delete original.readingSources; original.schemaVersion = 1;
  original.readingNotes = [{ id: ordinary.page.id, noteType: "annotation", body: "# literal\n\n**text**", color: "#123456",
    status: "trashed", isFavorite: true, revision: 7, createdAt: "2024-01-01", updatedAt: "2024-02-01", trashedAt: "2024-02-01",
    tagIds: ["tag"], projectIds: ["project"], source: { paperId: "deleted-paper", paperTitle: "Retained title", attachmentId: "pdf",
      contentSha256: "a".repeat(64), pageIndex: 3, rects: [], quoteExact: "Retained quote", quotePrefix: "", quoteSuffix: "", state: "unavailable" } }];
  storage.removeItem("research-assistant.notes.v1.indexeddb");
  storage.setItem("research-assistant.notes.v1", JSON.stringify(original));
  const reading = await browserNotesGateway.getReadingNote(noteBlockId(String(ordinary.page.id)));
  expect(reading.pageId).not.toBe(ordinary.page.id);
  expect(reading).toMatchObject({ revision: 7, status: "trashed", isFavorite: true, tagIds: ["tag"], projectIds: ["project"], createdAt: "2024-01-01" });
  const migrated = await browserNotesGateway.loadDocument(reading.pageId!);
  expect(migrated.document.blocks.map(block => block.type)).toEqual(["paragraph", "paragraph", "paragraph"]);
  expect(migrated.document.blocks[0]?.content).toMatchObject({ items: [{ text: "# literal", styles: [] }] });
  expect((await browserNotesGateway.loadDocument(ordinary.page.id)).page.title).toBe("Existing page");
  expect((await browserNotesGateway.getReadingNote(reading.id)).pageId).toBe(reading.pageId);
});

it("persists a position annotation without inventing a quote and retains its anchor through trash and restore", async () => {
  const rect = { x: .25, y: .6, width: .00001, height: .00001 };
  const created = await browserNotesGateway.createReadingNote({ itemId: "paper", attachmentId: "pdf", pageIndex: 2, pageLabel: "iii", rects: [rect], quoteExact: "", noteType: "annotation", body: "Position note", selector: { schemaVersion: 2, attachmentSha256: "", segments: [{ pageIndex: 2, exactText: "", rects: [rect] }] } });
  expect(created.source).toMatchObject({ pageIndex: 2, rects: [rect], quoteExact: "", state: "exact" });
  const reloaded = await browserNotesGateway.getReadingNote(created.id); expect(reloaded.source).toEqual(created.source);
  const trashed = await browserNotesGateway.setReadingNoteStatus({ id: created.id, status: "trashed", expectedRevision: reloaded.revision });
  const restored = await browserNotesGateway.setReadingNoteStatus({ id: created.id, status: "active", expectedRevision: trashed.revision });
  expect(restored.source).toEqual(created.source); expect(restored.body).toBe("Position note");
});
