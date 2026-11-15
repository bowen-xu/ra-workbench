// @vitest-environment happy-dom
import { BlockNoteEditor } from "@blocknote/core";
import { expect, it } from "vitest";
import { sourceBlockSelection } from "./sourceBlockSelection";

it("locates shared note Unicode across formatting and hard breaks without counting nested blocks", () => {
  const editor = BlockNoteEditor.create({initialContent: [{id: "b", type: "paragraph", content: [
    {type: "text", text: "A😀", styles: {bold: true}}, {type: "text", text: "B\nsecond", styles: {}},
  ], children: [{id: "child", type: "paragraph", content: "nested"}]}]});
  const document = editor.prosemirrorView.state.doc;
  const range = sourceBlockSelection(document, "b", {startLine: 1, startColumn: 2, endLine: 2, endColumn: 3});
  expect(document.textBetween(range.from, range.to, "\n", "\n")).toBe("😀B\nsec");
  expect(() => sourceBlockSelection(document, "b", {startLine: 3, startColumn: 1, endLine: 3, endColumn: 1})).toThrow("超出");
  expect(() => sourceBlockSelection(document, "missing", {startLine: 1, startColumn: 1, endLine: 1, endColumn: 1})).toThrow("不存在");
});
