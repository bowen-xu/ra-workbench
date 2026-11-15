// @vitest-environment happy-dom
import { BlockNoteEditor } from "@blocknote/core";
import { editorToBlocks } from "@research-assistant/core/notes";
import { expect, it } from "vitest";
import { baseBlockEditorSchema } from "./baseSchema";
import { buildNoteEditingItems } from "./noteEditingCommands";
import {TextSelection} from "@tiptap/pm/state";

it("offers exactly four heading levels and inserts H4 through the shared slash commands", () => {
  const editor = BlockNoteEditor.create({schema: baseBlockEditorSchema, initialContent: [{id: "p", type: "paragraph"}]}) as unknown as typeof baseBlockEditorSchema.BlockNoteEditor;
  expect(editor.schema.blockSchema.heading.propSchema.level.values).toEqual([1, 2, 3, 4]);
  const items = buildNoteEditingItems(editor);
  const headings = items.filter(item => "key" in item && String(item.key).startsWith("heading"));
  expect(headings.map(item => item.badge)).toEqual(["#", "##", "###", "####"]);
  headings[3]!.onItemClick();
  expect(editor.getBlock("p")).toMatchObject({type: "heading", props: {level: 4}});
});

it("provides portable commands and formula insertion without a Notes host", () => {
  const editor = BlockNoteEditor.create({ schema: baseBlockEditorSchema, initialContent: [{ id: "p", type: "paragraph", content: "Before " }] }) as unknown as typeof baseBlockEditorSchema.BlockNoteEditor;
  editor.setTextCursorPosition("p", "end");
  const items = buildNoteEditingItems(editor);
  expect(items.some(item => item.title === "行内公式")).toBe(true);
  expect(items.some(item => item.title === "公式块")).toBe(true);
  expect(items.some(item => "key" in item && item.key === "toggle_list")).toBe(true);
  expect(items.some(item => "key" in item && ["image", "audio"].includes(String(item.key)))).toBe(false);
  items.find(item => item.title === "行内公式")!.onItemClick();
  const blocks = editorToBlocks(editor.document);
  expect(JSON.stringify(blocks)).toContain("Before ");
  expect(JSON.stringify(blocks)).toContain("inlineFormula");
  expect(editor.document[0]!.id).toBe("p");
});

it.each(["", "中文", "text "])("inserts an inline formula without adding surrounding whitespace after %j", prefix => {
  const editor = BlockNoteEditor.create({schema: baseBlockEditorSchema, initialContent: [{id: "p", type: "paragraph", content: prefix + "后文"}]});
  const start = editor.prosemirrorState.selection.from;
  editor.transact(tr => tr.setSelection(TextSelection.create(tr.doc, start + prefix.length)));
  buildNoteEditingItems(editor).find(item => item.title === "行内公式")!.onItemClick();
  expect(editor.document[0]?.content).toEqual([
    ...(prefix ? [{type: "text", text: prefix, styles: {}}] : []),
    {type: "inlineFormula", props: {source: ""}},
    {type: "text", text: "后文", styles: {}},
  ]);
  expect(editor.prosemirrorState.selection.$from.nodeBefore?.type.name).toBe("inlineFormula");
});
