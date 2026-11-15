import { blockToNode } from "@blocknote/core";
import type { Node } from "@tiptap/pm/model";
import { Decoration } from "@tiptap/pm/view";
import type { compareTableRows } from "@research-assistant/core/hbs";
import type { NoteBlockV1, NoteDocumentV1 } from "@research-assistant/core/notes";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import { noteDocumentToBlockNote } from "../../features/notes/editor/blockNoteAdapter";
import { addedTextDecorations, oldBlocksWidget } from "../../features/notes/editor/noteDiffDecorations";
import "./NoteTableReview.css";

type TableBlock = NoteBlockV1 & { content: Extract<NoteBlockV1["content"], { kind: "table" }> };
interface Range { from: number; to: number; node: Node }
export type NoteTableComparison = typeof compareTableRows;
function snapshot(block: TableBlock, row: number, column: number): TableBlock {
  return { ...block, content: { ...block.content, rows: [[block.content.rows[row]![column]!]] } };
}
function tableNode(editor: NoteEditorInstance, block: TableBlock): Node {
  const document: NoteDocumentV1 = { schemaVersion: 1, pageId: "table-review" as NoteDocumentV1["pageId"], blocks: [block] };
  const node = blockToNode(noteDocumentToBlockNote(document)[0]!, editor.pmSchema, editor.schema.styleSchema);
  let table: Node | undefined;
  node.descendants(child => { if (child.type.name === "table") { table = child; return false; } return; });
  if (!table) throw new Error("MODEL_TABLE_RENDER_FAILED：真实表格节点不存在。");
  return table;
}

/** One review range, one live table. Detailed row/cell correspondence only
 * annotates the production editor; it never changes an HBS operation scope. */
export function tableReviewDecorations(editor: NoteEditorInstance, range: Range, before: TableBlock, after: TableBlock,
  compare: NoteTableComparison): Decoration[] {
  const result = compare(before.content.rows, after.content.rows), decorations: Decoration[] = [];
  if (result.ambiguous) {
    decorations.push(Decoration.node(range.from, range.to, { class: "note-diff-added-block" }));
    decorations.push(Decoration.widget(range.from, view => {
      const widget = oldBlocksWidget(editor, view.dom.ownerDocument, { id: "unmatched-table", before: [before], after: [after], afterFrom: 0 }, () => {}, host => {
        // Unknown row correspondence must not produce index-based text marks.
        host.querySelectorAll(".note-diff-deleted-text").forEach(mark => mark.replaceWith(...mark.childNodes));
      });
      return widget;
    }, { side: -3, key: `unmatched:${JSON.stringify([before, after])}`, ignoreSelection: true, stopEvent: () => true }));
    return decorations;
  }
  let table: Node | undefined, tableFrom = 0;
  range.node.descendants((node, offset) => {
    if (node.type.name === "table") { table = node; tableFrom = range.from + 1 + offset; return false; } return;
  });
  if (!table) return decorations;
  const oldTable = tableNode(editor, before), rows: Range[] = [];
  const columns = result.columns ?? after.content.rows[0]?.map((_cell, column) => ({ before: column, after: column })) ?? [];
  const previousColumn = new Map(columns.map(pair => [pair.after, pair.before]));
  table.forEach((node, offset) => rows.push({ node, from: tableFrom + 1 + offset, to: tableFrom + 1 + offset + node.nodeSize }));
  for (const [index, pair] of result.pairs.entries()) {
    if (pair.after === undefined) {
      const previousRow = pair.before!;
      const next = result.pairs.slice(index + 1).find(item => item.after !== undefined)?.after;
      const anchor = next === undefined ? tableFrom + table.nodeSize - 1 : rows[next]!.from;
      decorations.push(Decoration.widget(anchor, view => {
        const owner = view.dom.ownerDocument, row = owner.createElement("tr");
        row.className = "hbs-table-row-removed"; row.contentEditable = "false"; row.setAttribute("aria-label", "删除行");
        columns.forEach(pair => {
          const cell = owner.createElement("td");
          if (pair.before !== undefined) {
            const block = snapshot(before, previousRow, pair.before);
            cell.append(oldBlocksWidget(editor, owner, { id: `removed:${previousRow}:${pair.before}`, before: [block], after: [], afterFrom: 0 }, () => {}, undefined, "tableCell"));
          }
          row.append(cell);
        });
        return row;
      }, { side: -10 + index, key: `removed-row:${previousRow}:${JSON.stringify(before.content.rows[previousRow])}`, ignoreSelection: true, stopEvent: () => true }));
      continue;
    }
    const row = rows[pair.after]!;
    if (pair.before === undefined) {
      decorations.push(Decoration.node(row.from, row.to, { class: "hbs-table-row-added", "aria-label": "新增行" }));
      decorations.push(...addedTextDecorations(row, undefined));
      row.node.forEach((cell, offset, column) => {
        if (previousColumn.get(column) === undefined) decorations.push(Decoration.node(row.from + 1 + offset, row.from + 1 + offset + cell.nodeSize,
          { class: "hbs-table-column-added note-diff-added-block", "aria-label": "新增列" }));
      });
      continue;
    }
    const previousRow = pair.before, currentRow = pair.after;
    row.node.forEach((cell, offset, column) => {
      const from = row.from + 1 + offset, cellRange = { node: cell, from, to: from + cell.nodeSize };
      const oldColumn = previousColumn.get(column);
      if (oldColumn === undefined) {
        decorations.push(Decoration.node(from, cellRange.to, { class: "hbs-table-column-added note-diff-added-block", "aria-label": "新增列" }));
        decorations.push(...addedTextDecorations(cellRange, undefined));
        return;
      }
      const previous = oldTable.child(previousRow).child(oldColumn);
      if (cell.eq(previous)) return;
      decorations.push(...addedTextDecorations(cellRange, previous));
      cell.forEach((paragraph, position) => decorations.push(Decoration.node(from + 1 + position, from + 1 + position + paragraph.nodeSize,
        { class: "hbs-table-cell-current", "aria-label": "修改后的单元格" })));
      const oldBlock = snapshot(before, previousRow, oldColumn), newBlock = snapshot(after, currentRow, column);
      decorations.push(Decoration.widget(from + 1, view => oldBlocksWidget(editor, view.dom.ownerDocument,
        { id: `cell:${previousRow}:${column}`, before: [oldBlock], after: [newBlock], afterFrom: 0 }, () => {}, undefined, "tableCell"),
      { side: -3, key: `cell-old:${JSON.stringify([oldBlock, newBlock])}`, ignoreSelection: true, stopEvent: () => true }));
    });
  }
  return decorations;
}
