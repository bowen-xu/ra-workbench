import {
  decodeNoteDocument,
  createNoteDocumentFromText,
  noteBlockId,
  type NoteBlockV1,
  type NoteBlockId,
  type NoteDocumentV1,
  type NotePageId,
} from "@research-assistant/core/notes";

import {
  clearRecoveryDraft,
  createRecoveryDraft,
  loadRecoveryDraft,
  saveRecoveryDraft,
  type RecoveryDraftIdentity,
} from "../../../services/recoveryDraftStore";
import { decodeNoteReviewRecovery, type NoteReviewRecovery } from "../editor/noteReviewRecovery";

function noteRecoveryIdentity(pageId: NotePageId): RecoveryDraftIdentity {
  return { scope: "note-page", resourceId: pageId, slot: "document" };
}

function readingNoteRecoveryIdentity(id: NoteBlockId): RecoveryDraftIdentity {
  return { scope: "reading-note", resourceId: id, slot: "body" };
}

export interface NoteRecoveryDraft {
  readonly document: NoteDocumentV1;
  readonly baseRevision: number;
  readonly updatedAt: string;
  readonly review?: NoteReviewRecovery;
}

export function saveNoteRecoveryDraft(pageId: NotePageId, baseRevision: number, document: NoteDocumentV1, review?: NoteReviewRecovery): void {
  const identity = noteRecoveryIdentity(pageId), payload = review ? { document, review } : document;
  saveRecoveryDraft(createRecoveryDraft(identity, payload, { revision: baseRevision }));
  // Validate the staged record; asynchronous close guards also await its durable receipt.
  if (JSON.stringify(loadRecoveryDraft(identity)?.payload) !== JSON.stringify(payload)) throw new Error("恢复草稿写入未确认，请保留输入后重试。");
}

export function loadNoteRecoveryDraft(pageId: NotePageId): NoteRecoveryDraft | undefined {
  const envelope = loadRecoveryDraft(noteRecoveryIdentity(pageId));
  if (envelope === undefined) return undefined;
  if (envelope.baseRevision === undefined || !Number.isInteger(envelope.baseRevision) || envelope.baseRevision < 0) {
    throw new Error("笔记恢复草稿缺少有效的基础版本，原记录已保留");
  }
  const payload = envelope.payload;
  const bundled = payload && typeof payload === "object" && "review" in payload && "document" in payload ? payload : undefined;
  const document = decodeNoteDocument(bundled ? bundled.document : payload);
  if (document.pageId !== pageId) throw new Error("笔记恢复草稿身份不匹配，原记录已保留");
  return { document, baseRevision: envelope.baseRevision, updatedAt: envelope.updatedAt,
    ...(bundled ? { review: decodeNoteReviewRecovery(bundled.review, pageId) } : {}) };
}

export function clearNoteRecoveryDraft(pageId: NotePageId): void {
  clearRecoveryDraft(noteRecoveryIdentity(pageId));
}

export interface ReadingNoteRecoveryDraft {
  readonly body: string;
  readonly baseRevision: number;
  readonly updatedAt: string;
}

export function loadReadingNoteRecoveryDraft(id: NoteBlockId): ReadingNoteRecoveryDraft | undefined {
  const envelope = loadRecoveryDraft(readingNoteRecoveryIdentity(id));
  if (envelope === undefined) return undefined;
  if (envelope.baseRevision === undefined || !Number.isInteger(envelope.baseRevision) || envelope.baseRevision < 0
    || typeof envelope.payload !== "object" || envelope.payload === null
    || !("body" in envelope.payload) || typeof envelope.payload.body !== "string") {
    throw new Error("阅读笔记恢复草稿格式无效，原记录已保留");
  }
  return { body: envelope.payload.body, baseRevision: envelope.baseRevision, updatedAt: envelope.updatedAt };
}

export function clearReadingNoteRecoveryDraft(id: NoteBlockId): void {
  clearRecoveryDraft(readingNoteRecoveryIdentity(id));
}

function legacyAnnotationIdentity(pageId: NotePageId): RecoveryDraftIdentity {
  return { scope: "note-page", resourceId: pageId, slot: "legacy-annotation-document" };
}

function loadLegacyAnnotationDocument(pageId: NotePageId): NoteRecoveryDraft | undefined {
  const envelope = loadRecoveryDraft(legacyAnnotationIdentity(pageId));
  if (!envelope) return;
  const document = decodeNoteDocument(envelope.payload);
  if (document.pageId !== pageId || envelope.baseRevision === undefined || !Number.isSafeInteger(envelope.baseRevision) || envelope.baseRevision < 0) {
    throw new Error("旧批注恢复文档身份或版本无效，原记录已保留");
  }
  return { document, baseRevision: envelope.baseRevision, updatedAt: envelope.updatedAt };
}

/** Decoder-only bridge from old PDF envelopes; store an ordinary document,
 * never overwrite the active editor's primary recovery slot. */
export function stageLegacyAnnotationDocument(pageId: NotePageId, baseRevision: number, body: string): void {
  const existing = loadLegacyAnnotationDocument(pageId);
  const document = createNoteDocumentFromText(pageId, body, () => noteBlockId(crypto.randomUUID()));
  if (existing) {
    if (existing.baseRevision === baseRevision && JSON.stringify(existing.document.blocks.map(block => block.content)) === JSON.stringify(document.blocks.map(block => block.content))) return;
    throw new Error("已有另一份旧批注恢复文档，原 PDF 草稿已保留");
  }
  saveRecoveryDraft(createRecoveryDraft(legacyAnnotationIdentity(pageId), document, { revision: baseRevision }));
}

export function hasLegacyReadingRecoveryDraft(id: NoteBlockId, pageId: NotePageId): boolean {
  return loadReadingNoteRecoveryDraft(id) !== undefined || loadLegacyAnnotationDocument(pageId) !== undefined;
}

export function remapNoteBlocks(blocks: readonly NoteBlockV1[]): readonly NoteBlockV1[] {
  return blocks.map((block) => ({
    ...block,
    id: noteBlockId(crypto.randomUUID()),
    children: remapNoteBlocks(block.children),
  }));
}

/** One-way migration: only remove the legacy draft after the ordinary draft is durable. */
export function migrateReadingNoteRecoveryDraft(id: NoteBlockId, pageId: NotePageId): "migrated" | "collision" | undefined {
  const legacy = loadReadingNoteRecoveryDraft(id);
  const annotation = legacy ? undefined : loadLegacyAnnotationDocument(pageId);
  if (!legacy && !annotation) return;
  if (loadNoteRecoveryDraft(pageId)) return "collision";
  const document = legacy ? createNoteDocumentFromText(pageId, legacy.body, () => noteBlockId(crypto.randomUUID())) : annotation!.document;
  saveNoteRecoveryDraft(pageId, (legacy ?? annotation)!.baseRevision, document);
  if (legacy) clearReadingNoteRecoveryDraft(id);
  else clearRecoveryDraft(legacyAnnotationIdentity(pageId));
  return "migrated";
}
