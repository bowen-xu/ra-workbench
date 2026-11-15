import { closeHistory } from "@tiptap/pm/history";
import { decodeNoteDocument, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { blockNoteToNoteDocument, noteDocumentToBlockNote, retainNoteProperties } from "./blockNoteAdapter";
import type { NoteEditorInstance } from "./noteEditorSchema";
import { appendNoteReviewBaseline } from "./noteReviewHistory";
import {documentBodyBlocks} from "../../../components/block-editor/documentTitlePolicy";
import {documentTitleId} from "../../../components/block-editor/documentTitle";
import {replaceBlockText} from "../../../components/block-editor/blockTextEditing";

import type {NoteEditorInstallPort} from "@research-assistant/workbench-sdk/note-editor";
export type {NoteEditorInstallPort} from "@research-assistant/workbench-sdk/note-editor";

function canonical(value: unknown): string {
  return JSON.stringify(value, (_key, item: unknown) => item !== null && typeof item === "object" && !Array.isArray(item)
    ? Object.fromEntries(Object.entries(item).sort(([a], [b]) => a.localeCompare(b))) : item);
}
function fail(): never { throw new Error("笔记编辑器已变化或修改无法完整应用，请重新核对正文。"); }
const INLINE_TYPES = new Set(["paragraph", "heading", "bulletListItem", "numberedListItem", "checkListItem", "quote", "codeBlock"]);

/** Owns no saves or native receipts. The caller must hold the save coordinator's
 * external lease and verify the native receipt before invoking the installer.
 * Uses the original BlockNote instance; inline edits update blocks, structure
 * edits replace only their changed range. Neither path resets editor history.
 */
export function createNoteEditorInstallPort(options: {
  readonly editor: NoteEditorInstance;
  readonly pageId: NoteDocumentV1["pageId"];
  readonly canInstall: () => boolean;
  readonly canEdit?: () => boolean;
  /** Stamp the installed projection even when the view suppressed change
   * callbacks while read-only; unlocking must not publish it as manual input. */
  readonly onInstalled?: (document: NoteDocumentV1) => void;
  readonly getTrustedDocument?: () => NoteDocumentV1;
}): NoteEditorInstallPort {
  const { editor, pageId } = options;
  let disposed = false;
  let installing = false;
  const childPages = new Map<string, string>();
  const retainReferences = (blocks: readonly NoteBlockV1[]): void => blocks.forEach(block => {
    if (block.type === "childPage" && block.content.kind === "reference") childPages.set(block.id, block.content.targetId);
    retainReferences(block.children);
  });
  const read = () => {
    const trusted = options.getTrustedDocument?.(); if (trusted) retainReferences(trusted.blocks);
    const value = blockNoteToNoteDocument(pageId, editor.document, { validChildPages: [...childPages].map(([blockId, targetId]) => ({ blockId, targetId })) });
    return trusted ? retainNoteProperties(value,trusted,editor.schema.blockSchema) : value;
  };
  const available = () => !disposed && !installing && editor.isEditable && options.canInstall();
  return {
    setWriteLocked: locked => { editor.isEditable = !locked && (options.canEdit?.() ?? true); },
    get isInstalling() { return installing; },
    read,
    dispose() { disposed = true; },
    prepareStructure(baseline, candidate, review, installation) {
      if (disposed || installing || !options.canInstall() || baseline.pageId !== pageId || candidate.pageId !== pageId) return fail();
      retainReferences(baseline.blocks); retainReferences(candidate.blocks);
      const normalize = (blocks: ReturnType<typeof noteDocumentToBlockNote>): ReturnType<typeof noteDocumentToBlockNote> => blocks.map(block => {
        const schema = editor.schema.blockSchema[block.type as keyof typeof editor.schema.blockSchema]?.propSchema as Record<string, { default: unknown }> | undefined;
        const props = Object.fromEntries(Object.entries(schema ?? {}).map(([key, rule]) => [key, block.props?.[key as keyof typeof block.props] ?? rule.default]));
        return { ...block, props, children: normalize(block.children as ReturnType<typeof noteDocumentToBlockNote> ?? []) } as typeof block;
      });
      const expected = normalize(noteDocumentToBlockNote(decodeNoteDocument(candidate)));
      const actual = read();
      const emptyPlaceholder = (value: NoteDocumentV1) => value.blocks.length === 1 && value.blocks[0]?.type === "paragraph"
        && value.blocks[0].content.kind === "inline" && value.blocks[0].content.items.length === 0 && value.blocks[0].children.length === 0;
      const normalized = (value: NoteDocumentV1) => canonical(normalize(noteDocumentToBlockNote(value)));
      const matches = (expected: NoteDocumentV1, value: NoteDocumentV1) => (expected.title === undefined || value.title === expected.title)
        && (!expected.blocks.length && emptyPlaceholder(value) || normalized(value) === normalized(expected));
      // Accepted candidate text is already visible in the review projection.
      // A native saved receipt may therefore confirm that exact live document.
      if (!matches(baseline, read()) && !matches(candidate, read())) return fail();
      const before = normalize(noteDocumentToBlockNote(actual));
      const captured = canonical(read());
      let prefix = 0, suffix = 0;
      while (prefix < before.length && prefix < expected.length && canonical(before[prefix]) === canonical(expected[prefix])) prefix++;
      while (suffix < before.length - prefix && suffix < expected.length - prefix
        && canonical(before[before.length - suffix - 1]) === canonical(expected[expected.length - suffix - 1])) suffix++;
      let used = false;
      return () => {
        if (used || disposed || installing || !options.canInstall() || canonical(read()) !== captured) return fail();
        used = true; installing = true;
        try {
          const body = documentBodyBlocks(editor.document);
          const removed = body.slice(prefix, before.length - suffix);
          const added = expected.slice(prefix, expected.length - suffix);
          const state = editor.prosemirrorState;
          editor.transact(transaction => {
            closeHistory(transaction);
            if (candidate.title !== undefined && candidate.title !== actual.title && editor.getBlock(documentTitleId(pageId)))
              if (!replaceBlockText(transaction, documentTitleId(pageId), candidate.title)) return fail();
            if (removed.length) editor.replaceBlocks(removed, added);
            else if (added.length) {
              const anchor = body[prefix] ?? body.at(-1) ?? editor.getBlock(documentTitleId(pageId));
              if (!anchor) return fail();
              editor.insertBlocks(added, anchor, prefix < before.length ? "before" : "after");
            }
            if (review) appendNoteReviewBaseline(state, transaction, review.baseline, review);
            installation?.onTransaction?.(transaction);
            if (installation?.addToHistory !== undefined) transaction.setMeta("addToHistory", installation.addToHistory);
          });
          editor.transact(transaction => { closeHistory(transaction); });
          if (!matches(candidate, read())) return fail();
          options.onInstalled?.(candidate);
        } finally { installing = false; }
        return undefined;
      };
    },
    prepare(baselineInput, candidateInput) {
      if (!available()) return fail();
      // Freeze inputs and reject decoder normalization of unknown/lossy fields.
      const baseline = decodeNoteDocument(JSON.parse(JSON.stringify(baselineInput)));
      const candidate = decodeNoteDocument(JSON.parse(JSON.stringify(candidateInput)));
      if (baseline.pageId !== pageId || candidate.pageId !== pageId
        || canonical(baseline) !== canonical(baselineInput) || canonical(candidate) !== canonical(candidateInput)) return fail();
      const updates: ReturnType<typeof noteDocumentToBlockNote> = [];
      const compare = (before: readonly NoteBlockV1[], after: readonly NoteBlockV1[]) => {
        if (before.length !== after.length) return fail();
        before.forEach((block, index) => {
          const next = after[index];
          if (!next || block.id !== next.id || block.type !== next.type || canonical(block.props) !== canonical(next.props)) return fail();
          if (canonical(block.content) !== canonical(next.content)) {
            if (!INLINE_TYPES.has(block.type) || block.content.kind !== "inline" || next.content.kind !== "inline") return fail();
            updates.push(...noteDocumentToBlockNote({ ...candidate, blocks: [next] }));
          }
          compare(block.children, next.children);
        });
      };
      compare(baseline.blocks, candidate.blocks);
      const titleChanged = candidate.title !== undefined && baseline.title !== candidate.title;
      if (!updates.length && !titleChanged || updates.length > 32) return fail();
      retainReferences(baseline.blocks); retainReferences(candidate.blocks);
      // BlockNote supplies schema defaults absent from older persisted documents.
      // Accept only those known defaults; never ignore arbitrary prop differences.
      const matches = (expected: readonly NoteBlockV1[], actual: readonly NoteBlockV1[]): boolean => expected.length === actual.length
        && expected.every((block, index) => {
          const current = actual[index];
          if (!current || block.id !== current.id || block.type !== current.type || canonical(block.content) !== canonical(current.content)) return false;
          const defaults = editor.schema.blockSchema[block.type as keyof typeof editor.schema.blockSchema]?.propSchema;
          const props = { ...current.props };
          for (const key of Object.keys(props)) {
            if (!(key in block.props) && defaults && key in defaults
              && props[key] === (defaults as Record<string, { default: unknown }>)[key]?.default) delete props[key];
          }
          return canonical(block.props) === canonical(props) && matches(block.children, current.children);
        });
      if (!matches(baseline.blocks, read().blocks) || baseline.title !== undefined && baseline.title !== read().title) return fail();
      const captured = canonical(read());
      let consumed = false;
      return () => {
        if (consumed || !available() || canonical(read()) !== captured) return fail();
        consumed = true;
        installing = true;
        try {
          editor.transact((transaction) => {
            closeHistory(transaction);
            if (titleChanged && editor.getBlock(documentTitleId(pageId)) && !replaceBlockText(transaction, documentTitleId(pageId), candidate.title!)) return fail();
            for (const update of updates) {
              if (!update.id || update.content === undefined) return fail();
              editor.updateBlock(update.id, { content: update.content });
            }
          });
          // The next user keystroke must not merge into the accepted change.
          editor.transact((transaction) => { closeHistory(transaction); });
          if (!matches(candidate.blocks, read().blocks) || candidate.title !== undefined && candidate.title !== read().title) return fail();
          options.onInstalled?.(candidate);
        } finally { installing = false; }
        return undefined;
      };
    },
  };
}
