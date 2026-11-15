// @vitest-environment happy-dom

import { noteBlockId, notePageId, type NoteDocumentV1, createNoteDocumentFromText } from "@research-assistant/core/notes";
import { beforeEach, describe, expect, it } from "vitest";

import {
  clearNoteRecoveryDraft,
  loadNoteRecoveryDraft,
  loadReadingNoteRecoveryDraft,
  saveNoteRecoveryDraft,
  migrateReadingNoteRecoveryDraft,
  stageLegacyAnnotationDocument,
  hasLegacyReadingRecoveryDraft,
} from "./noteRecovery";

import { createRecoveryDraft, saveRecoveryDraft } from "../../../services/recoveryDraftStore";

class MemoryStorage implements Storage {
  readonly #values = new Map<string, string>();
  get length(): number { return this.#values.size; }
  clear(): void { this.#values.clear(); }
  getItem(key: string): string | null { return this.#values.get(key) ?? null; }
  key(index: number): string | null { return [...this.#values.keys()][index] ?? null; }
  removeItem(key: string): void { this.#values.delete(key); }
  setItem(key: string, value: string): void { this.#values.set(key, value); }
}

describe("note recovery drafts", () => {
  beforeEach(() => {
    Object.defineProperty(window, "localStorage", { configurable: true, value: new MemoryStorage() });
  });

  it("round-trips a Note Page document with its base revision", () => {
    const pageId = notePageId("page-1");
    const document: NoteDocumentV1 = { schemaVersion: 1, pageId, blocks: [] };
    saveNoteRecoveryDraft(pageId, 7, document);
    expect(loadNoteRecoveryDraft(pageId)).toMatchObject({ baseRevision: 7, document });
    clearNoteRecoveryDraft(pageId);
    expect(loadNoteRecoveryDraft(pageId)).toBeUndefined();
  });

  it("moves legacy literal text to an ordinary document without losing its conflict baseline", () => {
    const id = noteBlockId("reading-1"), pageId = notePageId("page-1");
    saveRecoveryDraft(createRecoveryDraft({ scope: "reading-note", resourceId: id, slot: "body" }, { body: "# literal\n\n**not markdown**" }, { revision: 3 }));
    migrateReadingNoteRecoveryDraft(id, pageId);
    const draft = loadNoteRecoveryDraft(pageId)!;
    expect(draft.baseRevision).toBe(3);
    expect(draft.document.blocks.map(block => block.content)).toEqual([
      { kind: "inline", items: [{ type: "text", text: "# literal", styles: [] }] },
      { kind: "inline", items: [] },
      { kind: "inline", items: [{ type: "text", text: "**not markdown**", styles: [] }] },
    ]);
    expect(loadReadingNoteRecoveryDraft(id)).toBeUndefined();
    migrateReadingNoteRecoveryDraft(id, pageId);
    expect(loadNoteRecoveryDraft(pageId)).toEqual(draft);
  });
  it("does not overwrite a newer document recovery draft", () => {
    const id = noteBlockId("reading-1"), pageId = notePageId("page-1");
    saveRecoveryDraft(createRecoveryDraft({ scope: "reading-note", resourceId: id, slot: "body" }, { body: "legacy" }, { revision: 3 }));
    saveNoteRecoveryDraft(pageId, 5, { schemaVersion: 1, pageId, blocks: [] });
    expect(migrateReadingNoteRecoveryDraft(id, pageId)).toBe("collision");
    expect(loadReadingNoteRecoveryDraft(id)?.body).toBe("legacy");
    expect(loadNoteRecoveryDraft(pageId)?.baseRevision).toBe(5);
  });
  it("retains a legacy PDF draft as a document without replacing live recovery input", () => {
    const id = noteBlockId("reading-1"), pageId = notePageId("page-1");
    saveNoteRecoveryDraft(pageId, 5, { schemaVersion: 1, pageId, blocks: [] });
    stageLegacyAnnotationDocument(pageId, 3, "# old PDF input");
    stageLegacyAnnotationDocument(pageId, 3, "# old PDF input");
    expect(hasLegacyReadingRecoveryDraft(id, pageId)).toBe(true);
    expect(migrateReadingNoteRecoveryDraft(id, pageId)).toBe("collision");
    expect(loadNoteRecoveryDraft(pageId)?.baseRevision).toBe(5);
    clearNoteRecoveryDraft(pageId);
    expect(migrateReadingNoteRecoveryDraft(id, pageId)).toBe("migrated");
    expect(loadNoteRecoveryDraft(pageId)?.baseRevision).toBe(3);
    expect(loadNoteRecoveryDraft(pageId)?.document.blocks[0]?.content).toEqual({ kind: "inline", items: [{ type: "text", text: "# old PDF input", styles: [] }] });
    expect(hasLegacyReadingRecoveryDraft(id, pageId)).toBe(false);
  });
});

it("round-trips the latest D, independent B and exact uncertain request together", () => {
  Object.defineProperty(window, "localStorage", { configurable: true, value: new MemoryStorage() });
  const pageId = notePageId("review-page");
  const document = createNoteDocumentFromText(pageId, "Agent plus latest manual input", () => noteBlockId("body"));
  const baseline = createNoteDocumentFromText(pageId, "Original body", () => noteBlockId("body"));
  const request = { pageId, conversationId: "conversation", proposalId: "proposal", generation: 2,
    requestId: "uncertain-id", action: "edit" as const, document: baseline };
  const review = { document, baseline, request, expectedDocument: baseline, proposalId: "proposal", conversationId: "conversation" };
  saveNoteRecoveryDraft(pageId, 8, document, review);
  expect(loadNoteRecoveryDraft(pageId)).toMatchObject({ baseRevision: 8, document, review });
});
it("retains a malformed review envelope and refuses cross-page request snapshots", () => {
  const storage = new MemoryStorage();
  Object.defineProperty(window, "localStorage", { configurable: true, value: storage });
  const pageId = notePageId("review-page"), document = createNoteDocumentFromText(pageId, "Current", () => noteBlockId("body"));
  const other = createNoteDocumentFromText(notePageId("other-page"), "Other", () => noteBlockId("body"));
  const review = { document, baseline: document, request: { pageId, conversationId: "conversation", proposalId: "proposal",
    generation: 2, requestId: "id", action: "edit" as const, document: other }, expectedDocument: document };
  saveNoteRecoveryDraft(pageId, 8, document, review);
  const raw = storage.getItem(storage.key(0)!);
  expect(() => loadNoteRecoveryDraft(pageId)).toThrow("正文或历史身份无效");
  expect(storage.getItem(storage.key(0)!)).toBe(raw);
});
it("does not report a durable recovery draft when storage silently drops it", () => {
  const storage = new MemoryStorage(); storage.setItem = () => {};
  Object.defineProperty(window, "localStorage", { configurable: true, value: storage });
  const pageId = notePageId("review-page");
  expect(() => saveNoteRecoveryDraft(pageId, 8, createNoteDocumentFromText(pageId, "Current", () => noteBlockId("body")))).toThrow("写入未确认");
});

it("formats structured and serialized native failures as readable recovery messages", async () => {
  const { noteReviewErrorMessage, isNoteReviewConflict } = await import("../editor/noteReviewRecovery");
  for (const reason of [{ status: "failed", message: "saved note changed" }, '{"status":"failed","message":"saved note changed"}']) {
    expect(noteReviewErrorMessage(reason)).toBe("笔记或审阅版本发生变化，当前输入已保留，请比较版本。");
    expect(isNoteReviewConflict(reason)).toBe(true);
  }
  expect(noteReviewErrorMessage({ status: "unknown" })).not.toContain("[object Object]");
});
