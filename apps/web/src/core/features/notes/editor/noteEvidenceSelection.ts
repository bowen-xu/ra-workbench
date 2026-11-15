import type { Node } from "@tiptap/pm/model";
import { formatSourceLink } from "@research-assistant/core/source";
import type { AgentEvidenceInput } from "@research-assistant/core/agent";

/** Capture each block's own text; never invent positions from string matching. */
export function noteEvidenceSelection(document: Node, pageId: string, from: number, to: number): readonly (AgentEvidenceInput & {readonly source: string})[] {
  const result: (AgentEvidenceInput & {readonly source: string})[] = [];
  document.descendants((block, position) => {
    if (typeof block.attrs.id !== "string" || !block.firstChild?.isTextblock) return true;
    const base = position + 2, characters: {text: string; from: number; to: number; row: number; column: number}[] = [];
    let row = 1, column = 1;
    block.firstChild.descendants((inline, offset) => {
      let cursor = base + offset;
      const text = inline.isText ? inline.text ?? "" : inline.type.name === "hardBreak" ? "\n" : inline.isLeaf && inline.isInline ? "\uFFFC" : "";
      for (const character of Array.from(text)) {
        characters.push({text: character, from: cursor, to: cursor + (inline.isText ? character.length : inline.nodeSize), row, column});
        if (character === "\n") { row++; column = 1; } else column++;
        cursor += character.length;
      }
      return false;
    });
    const selected = characters.filter(character => character.from >= from && character.to <= to);
    const first = selected.find(character => character.text !== "\n"), last = [...selected].reverse().find(character => character.text !== "\n");
    if (first && last) result.push({type: "annotation", comment: "", title: "笔记选区", text: selected.map(character => character.text).join(""), source: formatSourceLink({path: ["notes", pageId], locator: {kind: "block", blockId: block.attrs.id, range: {startLine: first.row, startColumn: first.column, endLine: last.row, endColumn: last.column}}})});
    return true;
  });
  return result;
}
