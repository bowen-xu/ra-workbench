import { blockToNode, createInternalHTMLSerializer } from "@blocknote/core";
import type { NoteBlockV1, NoteDocumentV1 } from "@research-assistant/core/notes";
import type { Node as ProseMirrorNode, Schema } from "@tiptap/pm/model";
import { Plugin, PluginKey, type EditorState } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";
import { noteDocumentToBlockNote } from "./blockNoteAdapter";
import { createEditorDiffChunkActions, updateEditorDiffChunkActions } from "../../../components/editor-diff/editorDiffChunkActions";
import type { NoteEditorInstance, PartialNoteEditorBlock } from "./noteEditorSchema";
import { highlightBlockCodeSnapshot } from "../../../components/block-editor/codeBlockSyntax";
import { highlightOldInline, noteInlineChanges } from "./noteInlineDiff";

// The pinned serializer's BlockSchema constraint cannot represent the optional
// default heading prop under exactOptionalPropertyTypes. Keep that upstream
// generic compatibility assertion at this rendering boundary.
const internalSerializer = createInternalHTMLSerializer as unknown as (schema: Schema, editor: NoteEditorInstance) => {
  serializeBlocks(blocks: PartialNoteEditorBlock[], options: { document: Document }): string;
};

export interface NoteEditorDiffChunk {
  readonly choice?: string;
  readonly id: string;
  readonly before: readonly NoteBlockV1[];
  readonly after: readonly NoteBlockV1[];
  /** Position among candidate root blocks, including pure deletions. */
  readonly afterFrom: number;
}
export interface NoteDiffPresentation {
  readonly conflict?: boolean;
  readonly chunks: readonly NoteEditorDiffChunk[];
  readonly disabled: boolean;
}
interface RootRange { readonly node: ProseMirrorNode; readonly from: number; readonly to: number }
export interface NoteDiffRange { readonly id: string; readonly from: number; readonly to: number }
interface DecorationState extends NoteDiffPresentation { readonly decorations: DecorationSet }

function rootRanges(document: ProseMirrorNode): RootRange[] {
  const roots: RootRange[] = [];
  document.firstChild?.forEach((node, offset) => roots.push({ node, from: offset + 1, to: offset + 1 + node.nodeSize }));
  return roots;
}

/** Derive coordinates from this editor's current document, never from DOM IDs. */
export function noteDiffRanges(state: EditorState, chunks: readonly NoteEditorDiffChunk[]): NoteDiffRange[] {
  const roots = rootRanges(state.doc);
  return chunks.map(chunk => {
    const added = roots.filter(root => chunk.after.some(block => block.id === root.node.attrs.id));
    const anchor = roots[chunk.afterFrom]?.from ?? roots.at(-1)?.to ?? 1;
    return { id: chunk.id, from: added[0]?.from ?? anchor, to: added.at(-1)?.to ?? anchor };
  });
}

function blockNodes(editor: NoteEditorInstance, blocks: readonly NoteBlockV1[]): ProseMirrorNode[] {
  const document: NoteDocumentV1 = { schemaVersion: 1, pageId: "diff-render" as NoteDocumentV1["pageId"], blocks: [...blocks] };
  return noteDocumentToBlockNote(document).map(block => blockToNode(block, editor.pmSchema, editor.schema.styleSchema));
}

function highlightOldText(root: HTMLElement, before: ProseMirrorNode | undefined, after: ProseMirrorNode | undefined): void {
  const table = root.querySelector("table");
  if (table) {
    Array.from(table.rows).forEach((row, rowIndex) => Array.from(row.cells).forEach((cell, cellIndex) => {
      const content = cell.querySelector("p");
      if (content) highlightOldInline(content,before?.maybeChild(rowIndex)?.maybeChild(cellIndex)?.firstChild??undefined,
        after?.maybeChild(rowIndex)?.maybeChild(cellIndex)?.firstChild??undefined);
    }));
  } else {
    const content = root.querySelector<HTMLElement>(".bn-inline-content");
    if (content) highlightOldInline(content,before,after);
  }
}

/** Whole block atoms have no text children. Match their real block identities
 * before snapshot IDs are stripped, including atoms inside nested blocks. */
function highlightOldBlockAtoms(root: HTMLElement, before: ProseMirrorNode, after: ProseMirrorNode | undefined): void {
  const current = new Map<string, ProseMirrorNode>();
  const collect = (node: ProseMirrorNode) => { if (node.attrs.id) current.set(String(node.attrs.id), node); };
  if (after) { collect(after); after.descendants(collect); }
  const removed = new Set<string>();
  const compare = (node: ProseMirrorNode) => {
    const atom = node.firstChild;
    const candidate = current.get(String(node.attrs.id))?.firstChild;
    if (node.attrs.id && atom?.isAtom && !atom.isInline && (!candidate || !atom.eq(candidate))) {
      removed.add(String(node.attrs.id));
    }
  };
  compare(before); before.descendants(compare);
  const blocks = [root, ...root.querySelectorAll<HTMLElement>("[data-id]")];
  for (const block of blocks) {
    if (removed.has(block.dataset.id ?? "")) block.querySelector(".bn-block-content")?.classList.add("note-diff-deleted-block-atom");
  }
}

export function addedTextDecorations(root: RootRange, previous: ProseMirrorNode | undefined): Decoration[] {
  const decorations: Decoration[] = [];
  const visit = (node: ProseMirrorNode, old: ProseMirrorNode | undefined, position: number) => {
    if (node.isAtom && !node.isInline) {
      if (!old || !node.eq(old)) decorations.push(Decoration.node(position, position + node.nodeSize, { class: "note-diff-added-block-atom" }));
      return;
    }
    if (node.inlineContent) {
      const changes = noteInlineChanges(old,node);
      node.forEach((child, offset) => {
        if (!child.isText) {
          if(child.isAtom&&changes.some(change=>change.fromB<offset+child.nodeSize&&offset<change.toB))
            decorations.push(Decoration.node(position+1+offset,position+1+offset+child.nodeSize,{class:"note-diff-added-atom"}));
          return;
        }
        for (const change of changes) {
          const from = Math.max(offset, change.fromB), to = Math.min(offset + child.nodeSize, change.toB);
          if (to > from) decorations.push(Decoration.inline(position + 1 + from,
            position + 1 + to, { class: "note-diff-added-text" }));
        }
      });
      return;
    }
    node.forEach((child, offset, index) => {
      const prior = child.attrs.id ? Array.from({ length: old?.childCount ?? 0 }, (_, i) => old?.child(i))
        .find(item => item?.attrs.id === child.attrs.id) : old?.maybeChild(index);
      visit(child, prior?.type === child.type ? prior : undefined, position + 1 + offset);
    });
  };
  visit(root.node, previous, root.from);
  return decorations;
}

export function oldBlocksWidget(editor: NoteEditorInstance, owner: Document, chunk: NoteEditorDiffChunk, activate: (id: string) => void,
  onRendered?: (widget: HTMLElement, chunk: NoteEditorDiffChunk) => void, embeddedType?: "codeBlock" | "quote" | "tableCell"): HTMLElement {
  const widget = owner.createElement(embeddedType && embeddedType !== "tableCell" ? "span" : "div");
  widget.className = embeddedType ? `note-diff-deleted-content note-diff-deleted-${embeddedType}` : "note-diff-deleted-blocks";
  widget.dataset.noteDiffChunk = chunk.id;
  const firstCandidate = chunk.after[0];
  if (firstCandidate && !embeddedType) widget.dataset.blockReviewAnchorFor = firstCandidate.id;
  widget.contentEditable = "false";
  widget.setAttribute("aria-label", "删除内容");
  widget.addEventListener("pointerdown", () => activate(chunk.id));
  const snapshot: NoteDocumentV1 = { schemaVersion: 1, pageId: "diff-render" as NoteDocumentV1["pageId"], blocks: [...chunk.before] };
  // The pinned upstream serializer calls the production block/inline renderers
  // through this editor's React context. It preserves nested blocks, marks,
  // object references, tables and formulas without creating another editor.
  // React custom renderers flush their contextual portals synchronously. Run
  // serialization after the host effect/transaction finished, never inside a
  // React lifecycle; widget descendants are presentation-owned DOM.
  queueMicrotask(() => {
    if (!widget.isConnected) return;
    widget.innerHTML = internalSerializer(editor.pmSchema, editor)
      .serializeBlocks(noteDocumentToBlockNote(snapshot, editor.schema.blockSchema), { document: owner });
    const candidates = blockNodes(editor, chunk.after), originals = blockNodes(editor, chunk.before);
    const roots = widget.querySelector(".bn-block-group")?.children;
    if (roots) Array.from(roots).forEach((root, index) => {
      const original = chunk.before[index];
      const candidate = candidates.find(node => node.attrs.id === original?.id);
      if (root instanceof HTMLElement) {
        const code = root.querySelector<HTMLElement>("pre > code");
        if (code) highlightBlockCodeSnapshot(code, String(original?.props.language ?? "text"));
        const originalNode = originals.find(node => node.attrs.id === original?.id);
        if (originalNode) highlightOldBlockAtoms(root, originalNode, candidate);
        highlightOldText(root,originalNode?.firstChild??undefined,candidate?.firstChild ?? undefined);
      }
    });
    if (embeddedType) {
      // Reuse the production renderer's rich inline DOM, inside the live
      // block/cell's contentDOM. Retain its inline nodes without nesting another
      // code/quote block or table inside the live editor.
      const content = widget.querySelector(embeddedType === "codeBlock" ? "pre > code"
        : embeddedType === "tableCell" ? "table tr > td, table tr > th" : "blockquote");
      if (content) widget.replaceChildren(...Array.from(content.childNodes));
    }
    // Deleted snapshots are not live BlockNote nodes. Retain renderer classes
    // while preventing ID-based side controls and navigation from finding the
    // old DOM before the candidate node with the same block identity.
    widget.querySelectorAll<HTMLElement>("[data-id]").forEach(element => element.removeAttribute("data-id"));
    // Upstream hover hit-testing treats this marker as a live draggable node
    // and then reads its ID. Serialized snapshots have neither editor position
    // nor ID, so retain visual classes without that live-node identity marker.
    widget.querySelectorAll<HTMLElement>('[data-node-type="blockContainer"]').forEach(element => element.removeAttribute("data-node-type"));
    widget.querySelectorAll<HTMLElement>("[contenteditable]").forEach(element => { element.contentEditable = "false"; });
    widget.querySelectorAll<HTMLButtonElement | HTMLInputElement | HTMLSelectElement>("button,input,select").forEach(control => { control.disabled = true; });
    onRendered?.(widget, chunk);
  });
  widget.addEventListener("click", event => {
    if (event.target instanceof Element && event.target.closest("a")) event.preventDefault();
  });
  return widget;
}

function chunkActions(owner: Document, chunk: NoteEditorDiffChunk, number: number, disabled: boolean,
  resolve: (action: "keep" | "undo", chunkId: string) => void, onBoth?: (id: string) => void): HTMLElement {
  const row = createEditorDiffChunkActions({ document: owner, number, disabled,
    ...(onBoth ? { conflict: { choice: chunk.choice, onBoth: () => onBoth(row.dataset.noteDiffChunk!) } } : {}),
    onResolve: action => { const current = row.dataset.noteDiffChunk; if (current) resolve(action, current); } });
  row.classList.add("note-diff-chunk-actions");
  row.dataset.noteDiffChunk = chunk.id;
  row.dataset.noteDiffActionKey = actionIdentity(chunk);
  row.contentEditable = "false";
  return row;
}

// Native chunk IDs authorize an exact content generation; they intentionally
// change on edits. Presentation identity follows the same block range instead.
function actionIdentity(chunk: NoteEditorDiffChunk): string {
  return JSON.stringify([chunk.before.map(block => block.id), chunk.after.map(block => block.id), chunk.choice]);
}

function buildDecorations(editor: NoteEditorInstance, state: EditorState, presentation: NoteDiffPresentation,
  callbacks: {
    readonly onBoth?: (id: string) => void;
    readonly onResolve: (action: "keep" | "undo", chunkId: string) => void;
    readonly onSelection: (chunkId: string | undefined) => void;
    readonly onOldWidgetRendered?: (widget: HTMLElement, chunk: NoteEditorDiffChunk) => void;
    readonly onOldWidgetDestroyed?: (widget: HTMLElement) => void;
  }): DecorationSet {
  const roots = rootRanges(state.doc);
  const ranges = noteDiffRanges(state, presentation.chunks);
  const decorations: Decoration[] = [];
  presentation.chunks.forEach((chunk, index) => {
    const range = ranges[index]!;
    const first = chunk.after[0], original = chunk.before[0];
    const embeddedType = chunk.before.length === 1 && chunk.after.length === 1 && first?.id === original?.id
      && first?.type === original?.type && !first?.children.length && !original?.children.length
      && (first?.type === "codeBlock" || first?.type === "quote") ? first.type : undefined;
    if (chunk.before.length) decorations.push(Decoration.widget(range.from + (embeddedType ? 2 : 0),
      view => oldBlocksWidget(editor, view.dom.ownerDocument, chunk, callbacks.onSelection, callbacks.onOldWidgetRendered, embeddedType), {
      side: -2, key: `old:${chunk.id}:${JSON.stringify([chunk.before, chunk.after])}`, ignoreSelection: true, stopEvent: () => true,
      destroy: widget => { if (widget instanceof HTMLElement) callbacks.onOldWidgetDestroyed?.(widget); },
    }));
    const oldNodes = blockNodes(editor, chunk.before);
    chunk.after.forEach(block => {
      const root = roots.find(root => root.node.attrs.id === block.id);
      if (!root) return;
      decorations.push(Decoration.node(root.from, root.to, { class: `note-diff-added-block${embeddedType ? " note-diff-embedded-block" : ""}`, "data-note-diff-chunk": chunk.id,
        ...(block === first && chunk.before.length && !embeddedType ? { "data-block-review-anchor": block.id } : {}) }));
      decorations.push(...addedTextDecorations(root, oldNodes.find(node => node.attrs.id === block.id)));
    });
    decorations.push(Decoration.widget(range.to, view => chunkActions(view.dom.ownerDocument, chunk, index + 1, presentation.disabled, callbacks.onResolve, presentation.conflict ? callbacks.onBoth : undefined), {
      side: -1, key: `actions:${actionIdentity(chunk)}`, ignoreSelection: true, stopEvent: () => true,
    }));
  });
  return DecorationSet.create(state.doc, decorations);
}

/** Presentation only: no transaction ever changes the document or baseline. */
export function createNoteDiffPlugin(editor: NoteEditorInstance, presentation: NoteDiffPresentation,
  callbacks: {
    readonly onBoth?: (id: string) => void;
    readonly onResolve: (action: "keep" | "undo", chunkId: string) => void;
    readonly onSelection: (chunkId: string | undefined) => void;
    readonly onOldWidgetRendered?: (widget: HTMLElement, chunk: NoteEditorDiffChunk) => void;
    readonly onOldWidgetDestroyed?: (widget: HTMLElement) => void;
  }): { readonly plugin: Plugin<DecorationState>; readonly key: PluginKey<DecorationState> } {
  const key = new PluginKey<DecorationState>("note-inline-diff");
  return { key, plugin: new Plugin<DecorationState>({
    key,
    state: {
      init: (_, state) => ({ ...presentation, decorations: buildDecorations(editor, state, presentation, callbacks) }),
      apply: (transaction, value, _oldState, state) => {
        const replacement = transaction.getMeta(key) as NoteDiffPresentation | undefined;
        const current = replacement ?? value;
        return transaction.docChanged || replacement
          ? { ...current, decorations: buildDecorations(editor, state, current, callbacks) }
          : value;
      },
    },
    props: { decorations: state => key.getState(state)?.decorations ?? DecorationSet.empty },
    view: () => ({ update: view => {
      const current = key.getState(view.state);
      const actions = new Map(current?.chunks.map((chunk, index) => [actionIdentity(chunk), { chunk, number: index + 1 }]) ?? []);
      view.dom.querySelectorAll<HTMLElement>(".note-diff-chunk-actions").forEach(row => {
        const value = actions.get(row.dataset.noteDiffActionKey ?? ""); if (!value) return;
        if (row.dataset.noteDiffChunk !== value.chunk.id) row.dataset.noteDiffChunk = value.chunk.id;
        updateEditorDiffChunkActions(row, { number: value.number, disabled: current?.disabled ?? false });
      });
      const cursor = view.state.selection.head;
      callbacks.onSelection(noteDiffRanges(view.state, current?.chunks ?? []).find(range => range.from <= cursor && cursor <= range.to)?.id);
    } }),
  }) };
}
