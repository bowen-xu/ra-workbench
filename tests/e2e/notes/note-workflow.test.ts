import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { noteBlockId } from "@research-assistant/core/notes";

import { NotesClient } from "../../../apps/web/src/core/features/notes/NotesClient";
import { browserNotesGateway } from "../../../apps/web/src/core/features/notes/gateways/browserNotesGateway";

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
  vi.stubGlobal("window", Object.assign(new EventTarget(), { localStorage: storage }));
});
afterEach(() => vi.unstubAllGlobals());

describe("unified note workflow", () => {
  it("persists page editing and shares one reading-note lifecycle across entry points", async () => {
    window.localStorage.setItem("research-assistant.literature.v1", JSON.stringify([{
      id: "paper-1", title: "Traceable result", attachments: [{ id: "pdf-1", contentSha256: "a".repeat(64) }],
    }]));
    const client = new NotesClient(browserNotesGateway);
    const readerChanges: number[] = [];
    const workspaceChanges: number[] = [];
    client.subscribe(() => readerChanges.push(client.getVersion()));
    client.subscribe(() => workspaceChanges.push(client.getVersion()));

    const parent = await client.mutate((gateway) => gateway.createPage({ title: "研究记录" }));
    const child = await client.mutate((gateway) => gateway.createPage({ title: "结果", parentPageId: parent.page.id }));
    const document = {
      ...child.document,
      blocks: [{
        id: child.document.blocks[0]!.id, type: "paragraph" as const, props: {},
        content: { kind: "inline" as const, items: [{ type: "text" as const, text: "可复现结果", styles: [] }] }, children: [],
      }],
    };
    const saved = await client.mutate((gateway) => gateway.saveDocument({ document, expectedRevision: 0, saveReason: "autosave" }));
    expect((await browserNotesGateway.loadDocument(child.page.id)).document).toEqual(saved.document);

    const notes = await Promise.all([
      ["highlight", "", "quoted"] as const,
      ["annotation", "comment", "quoted"] as const,
      ["page_note", "page comment", ""] as const,
    ].map(async ([noteType, body, quoteExact], pageIndex) => client.mutate((gateway) => gateway.createReadingNote({
      noteType, body, itemId: "paper-1", attachmentId: "pdf-1", pageIndex, rects: [], quoteExact,
    }))));
    const readingDocument = await browserNotesGateway.loadDocument(notes[1]!.pageId!);
    await client.mutate(gateway => gateway.saveDocument({
      document: { ...readingDocument.document, blocks: [{
        ...readingDocument.document.blocks[0]!,
        content: { kind: "inline", items: [{ type: "text", text: "revised", styles: [] }] },
      }] },
      expectedRevision: readingDocument.page.revision, saveReason: "autosave",
    }));
    const updated = await browserNotesGateway.getReadingNote(notes[1]!.id);
    const created = await client.mutate(gateway => gateway.createPage({ title: "阅读整理" }));
    const derived = await client.mutate(gateway => gateway.saveDocument({
      document: { ...created.document, blocks: notes.map(note => ({
        id: noteBlockId(crypto.randomUUID()), type: "paragraph" as const, props: {}, children: [],
        content: { kind: "inline" as const, items: [{ type: "mention" as const, targetKind: "reading_note" as const,
          targetId: note.id, labelSnapshot: "阅读笔记" }] },
      })) }, expectedRevision: created.page.revision, saveReason: "manual",
    }));
    const outgoing = await browserNotesGateway.listOutgoing(derived.page.id);
    expect(outgoing).toHaveLength(3);
    expect(new Set(outgoing.map((link) => link.relation))).toEqual(new Set(["mention"]));
    for (const note of notes) {
      expect(outgoing.filter(link => link.target.kind === "reading_note" && link.target.id === note.id)
        .map(link => link.relation).sort()).toEqual(["mention"]);
    }
    expect((await browserNotesGateway.listBacklinks("reading_note", updated.id))).toHaveLength(1);

    const trashed = await client.mutate((gateway) => gateway.setReadingNoteStatus({ id: updated.id, status: "trashed", expectedRevision: updated.revision }));
    expect((await browserNotesGateway.listReadingNotes()).map((note) => note.id)).not.toContain(updated.id);
    const restored = await client.mutate((gateway) => gateway.setReadingNoteStatus({ id: trashed.id, status: "active", expectedRevision: trashed.revision }));
    expect(restored).toEqual(expect.objectContaining({ id: updated.id, body: "revised" }));
    expect(readerChanges).toEqual(workspaceChanges);

    // This storage-only adapter has no durable search port. The same edit /
    // trash / restore search contract runs on real IndexedDB in
    // parent tests/acceptance/ui/note-manager-runtime-baseline.pw.ts.
    await expect(browserNotesGateway.searchNotes({search: "revised", paperId: "paper-1", noteType: "annotation"}))
      .rejects.toMatchObject({code: "storage_error", message: expect.stringContaining("note_search_adapter_unavailable")});
    expect((await browserNotesGateway.listPages()).find((page) => page.id === parent.page.id)).toBeDefined();
  });

  it("keeps a deleted image recoverable from pinned history without inline file deletion", async () => {
    const client = new NotesClient(browserNotesGateway);
    const asset = await browserNotesGateway.importNoteAsset({
      displayName: "figure.png",
      mimeType: "image/png",
      bytes: [137, 80, 78, 71],
    });
    const created = await client.mutate((gateway) => gateway.createPage({ title: "Asset history" }));
    const withImage = {
      ...created.document,
      blocks: [{
        id: noteBlockId("image-block"),
        type: "image" as const,
        props: {},
        content: {
          kind: "asset" as const,
          assetId: asset.id,
          displayName: asset.displayName,
          mimeType: asset.mimeType,
          caption: "",
        },
        children: [],
      }],
    };
    const inserted = await client.mutate((gateway) => gateway.saveDocument({
      document: withImage,
      expectedRevision: 0,
      saveReason: "manual",
    }));
    const removed = await client.mutate((gateway) => gateway.saveDocument({
      document: { ...inserted.document, blocks: [] },
      expectedRevision: 1,
      saveReason: "manual",
    }));
    const history = await browserNotesGateway.listRevisions(created.page.id);
    const imageRevision = history.find((revision) => revision.pageRevision === 1);
    if (imageRevision === undefined) throw new Error("missing image revision");
    const pinned = await browserNotesGateway.setRevisionPinned(created.page.id, imageRevision.id, true);
    expect(pinned.isPinned).toBe(true);
    expect([...await browserNotesGateway.readNoteAsset(asset.id)]).toEqual([137, 80, 78, 71]);

    const trashed = await browserNotesGateway.setPageStatus({
      id: created.page.id,
      status: "trashed",
      expectedRevision: removed.page.revision,
    });
    const report = await browserNotesGateway.permanentlyDeletePage({
      id: created.page.id,
      expectedRevision: trashed.revision,
    });
    expect(report.scheduledAssetCount).toBe(0);
    expect([...await browserNotesGateway.readNoteAsset(asset.id)]).toEqual([137, 80, 78, 71]);
  });
});
