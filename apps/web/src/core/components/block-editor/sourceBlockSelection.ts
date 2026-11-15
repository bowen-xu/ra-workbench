import type { Node } from "@tiptap/pm/model";
import type { SourceLineRange } from "@research-assistant/core/source";

/** Shared note coordinates cover only the block's own text, never child blocks.
 * Inline atoms occupy one object-replacement character; hard breaks start a row. */
export function sourceBlockSelection(document: Node, blockId: string, range: SourceLineRange): { from: number; to: number } {
  let result: { from: number; to: number } | undefined;
  document.descendants((block, position) => {
    if (block.attrs.id !== blockId) return true;
    const content = block.firstChild;
    if (!content?.isTextblock) throw new Error("该笔记块没有可按行列定位的正文");
    const base = position + 2;
    const rows: { from: number; to: number }[][] = [[]];
    content.descendants((inline, offset) => {
      if (inline.isText) {
        let cursor = base + offset;
        for (const character of Array.from(inline.text ?? "")) {
          if (character === "\n") rows.push([]);
          else rows.at(-1)!.push({ from: cursor, to: cursor + character.length });
          cursor += character.length;
        }
      } else if (inline.isLeaf && inline.isInline) {
        if (inline.type.name === "hardBreak") rows.push([]);
        else rows.at(-1)!.push({ from: base + offset, to: base + offset + inline.nodeSize });
      }
      return false;
    });
    const first = rows[range.startLine - 1]?.[range.startColumn - 1];
    const last = rows[range.endLine - 1]?.[range.endColumn - 1];
    if (!first || !last || first.from > last.from) throw new Error("来源行列超出当前笔记块，正文可能已变化");
    result = { from: first.from, to: last.to };
    return false;
  });
  if (!result) throw new Error("来源笔记块不存在或已被删除");
  return result;
}
