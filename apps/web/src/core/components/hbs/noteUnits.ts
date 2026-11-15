import type { DocumentChange, DocumentPath } from "@research-assistant/core/hbs";
import type {NoteBlockV1, NoteDocumentV1} from "@research-assistant/core/notes";
import {documentTitleId} from "../block-editor/documentTitle";
function titleProjection(document: NoteDocumentV1): NoteBlockV1 | undefined {
  return document.title === undefined ? undefined : {id: documentTitleId(document.pageId) as NoteBlockV1["id"], type: "heading", props: {level: 1},
    content: {kind: "inline", items: [{type: "text", text: document.title, styles: []}]}, children: []};
}
export function pathOverlaps(a: DocumentPath, b: DocumentPath): boolean {
  return a.slice(0, Math.min(a.length, b.length)).every((part, index) => JSON.stringify(part) === JSON.stringify(b[index]));
}
export function blockForPath(document: NoteDocumentV1, path: DocumentPath): NoteBlockV1 | undefined {
  if (path[0] === "title") return titleProjection(document);
  let siblings = document.blocks, block: NoteBlockV1 | undefined;
  for (const part of path) {
    if (typeof part !== "object" || part.key !== "id") continue;
    block = siblings.find(candidate => candidate.id === part.value);
    if (!block) return;
    siblings = block.children;
  }
  return block;
}
/** Snapshot fragments are real domain nodes inside their legal parent. A cell
 * is shown in a table, never as sliced HTML or a detached cell DOM element. */
export function noteUnitSnapshot(document: NoteDocumentV1, change: DocumentChange, side: "before" | "after"): readonly NoteBlockV1[] {
  const path = side === "before" ? change.beforePath : change.path;
  const block = blockForPath(document, path);
  // Only explicit document/collection paths own complete collections. A
  // missing ID or an unrendered metadata leaf never implies the whole note.
  if (!path.length) {const title = titleProjection(document); return title ? [title, ...document.blocks] : document.blocks;}
  if (path.length === 1 && path[0] === "blocks") return document.blocks;
  if (path.at(-1) === "children") return block?.children ?? [];
  if (!block) return [];
  if (block.content.kind !== "table") return [block];
  const rowsIndex = path.lastIndexOf("rows"), row = path[rowsIndex + 1], col = path[rowsIndex + 2];
  const axis = change.axis;
  if (axis) {
    const from = side === "before" ? axis.beforeFrom : axis.afterFrom, count = side === "before" ? axis.beforeCount : axis.afterCount;
    const rows = axis.direction === "row" ? block.content.rows.slice(from, from + count)
      : block.content.rows.map(row => row.slice(from, from + count));
    return rows.length && rows[0]?.length ? [{ ...block, content: { ...block.content, rows } }] : [];
  }
  if (rowsIndex >= 0 && typeof row === "number" && typeof col === "number") {
    const cell = block.content.rows[row]?.[col];
    return cell ? [{ ...block, content: { ...block.content, rows: [[cell]] } }] : [];
  }
  return [block];
}
