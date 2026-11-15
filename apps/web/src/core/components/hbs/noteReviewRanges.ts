import type { Node } from "@tiptap/pm/model";
import type { EditorState } from "@tiptap/pm/state";
import type { DocumentChange } from "@research-assistant/core/hbs";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import { blockForPath } from "./noteUnits";
import type { NoteReviewAnchor } from "./noteReviewProjection";

export interface NoteReviewRange { readonly from: number; readonly to: number; readonly node: Node }

/** Translate an explicit display path to native coordinates. Missing targets
 * stay missing; only an explicit document path may select all root blocks. */
export function noteUnitRanges(state: EditorState, document: NoteDocumentV1, change: DocumentChange): NoteReviewRange[] {
  const block = blockForPath(document, change.path), ranges: NoteReviewRange[] = [];
  const whole = !change.path.length || change.path.length === 1 && change.path[0] === "blocks";
  const children = change.path.at(-1) === "children" ? new Set(block?.children.map(child => child.id)) : undefined;
  if (!whole && !block) return ranges;
  state.doc.descendants((node, position) => {
    if (node.type.name !== "blockContainer") return;
    if (change.path[0] === "blocks" && node.firstChild?.type.name === "documentTitle") return false;
    if (children ? !children.has(node.attrs.id) : !whole && node.attrs.id !== block?.id) return;
    const rowsAt = change.path.lastIndexOf("rows");
    if (whole || children || block?.content.kind !== "table" || rowsAt < 0) {
      ranges.push({ from: position, to: position + node.nodeSize, node }); return false;
    }
    const row = change.path[rowsAt + 1], col = change.path[rowsAt + 2];
    if (!change.axis && typeof row !== "number") { ranges.push({ from: position, to: position + node.nodeSize, node }); return false; }
    node.descendants((table, tableOffset) => {
      if (table.type.name !== "table") return;
      table.forEach((rowNode, rowOffset, rowIndex) => rowNode.forEach((cell, cellOffset, colIndex) => {
        const axis = change.axis;
        const selected = axis ? (axis.direction === "row" ? rowIndex : colIndex) >= axis.afterFrom
          && (axis.direction === "row" ? rowIndex : colIndex) < axis.afterFrom + axis.afterCount : rowIndex === row && colIndex === col;
        if (selected) { const from = position + 1 + tableOffset + 1 + rowOffset + 1 + cellOffset; ranges.push({ from, to: from + cell.nodeSize, node: cell }); }
      }));
      return false;
    });
    if (!ranges.length) ranges.push({ from: position, to: position + node.nodeSize, node });
    return false;
  });
  return ranges;
}

export function noteAnchorPosition(state: EditorState, document: NoteDocumentV1, anchor: NoteReviewAnchor): number | undefined {
  let result: number | undefined;
  const parent = blockForPath(document, anchor.parentPath);
  state.doc.descendants((node, position) => {
    if (node.type.name !== "blockContainer") return;
    if (anchor.beforeId && node.attrs.id === anchor.beforeId) { result = position; return false; }
    if (!anchor.beforeId && parent && node.attrs.id === parent.id) {
      result = position + node.nodeSize - 1;
      node.forEach((child, offset) => {
        if (child.type.name === "blockGroup") result = position + 1 + offset + child.nodeSize - 1;
      });
      return false;
    }
    return;
  });
  if (result === undefined && anchor.parentPath.length === 1 && anchor.parentPath[0] === "blocks") return Math.max(1, state.doc.content.size - 1);
  return result;
}
