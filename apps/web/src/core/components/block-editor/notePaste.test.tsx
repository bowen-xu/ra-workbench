// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {afterEach, expect, it, vi} from "vitest";
import {NoteEditorSurface} from "./NoteEditorSurface";
import {baseBlockEditorSchema} from "./baseSchema";
import {noteEditorSchema} from "../../features/notes/editor/noteEditorSchema";
import {writingNoteSchema} from "../../../../../../plugins/writing_projects/src/features/writing/notes/writingNoteSchema";
import {blockNoteToNoteDocument} from "../../features/notes/editor/blockNoteAdapter";
import type {NoteEditorBlock} from "../../features/notes/editor/noteEditorSchema";
import {notePageId} from "@research-assistant/core/notes";
import {editorToBlocks} from "@research-assistant/core/notes";
import {decodeWritingNote} from "@research-assistant/writing";
import {closeHistory} from "@tiptap/pm/history";
import {TextSelection} from "@tiptap/pm/state";
import {blockEditorDefaults} from "./defaults";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const cleanup of cleanups.splice(0)) await cleanup(); vi.unstubAllGlobals();});

async function fixture(kind: "notes" | "writing", text = "", readOnly = false) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const editor = BlockNoteEditor.create({...blockEditorDefaults, schema: kind === "notes" ? noteEditorSchema : writingNoteSchema,
    initialContent: [{id: "body", type: "paragraph", content: text}],
  }) as unknown as typeof baseBlockEditorSchema.BlockNoteEditor;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(<NoteEditorSurface editor={editor} onChange={() => undefined} readOnly={readOnly}/>));
  await act(async () => {editor.setTextCursorPosition("body", "start"); editor.focus();});
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  const paste = async (formats: Readonly<Record<string, string>>) => {
    const event = new Event("paste", {bubbles: true, cancelable: true});
    Object.defineProperty(event, "clipboardData", {value: {types: Object.keys(formats), files: [], items: [],
      getData: (type: string) => formats[type] ?? ""}});
    await act(async () => editor.prosemirrorView.dom.dispatchEvent(event));
    return event;
  };
  return {editor, host, paste};
}

it.each(["notes", "writing"] as const)("pastes deeper external headings into %s as H4 while retaining their text", async kind => {
  const {editor, paste} = await fixture(kind);
  await paste({"text/html": "<h4>四级</h4><h5>五级来源</h5><h6>六级来源</h6>", "text/plain": "四级\n五级来源\n六级来源"});
  expect(editor.document.map(block => ({type: block.type, level: "level" in block.props ? block.props.level : undefined}))).toEqual([
    {type: "heading", level: 4}, {type: "heading", level: 4}, {type: "heading", level: 4},
  ]);
  expect(editor.prosemirrorState.doc.textContent).toBe("四级五级来源六级来源");
});

it.each(["notes", "writing"] as const)("pastes ordinary browser rich text into %s body", async kind => {
  const {editor, paste} = await fixture(kind);
  await paste({"text/html": '<meta charset="utf-8"><p>网页<strong>粗体</strong>与<em>斜体</em>，<a href="https://example.org/paper">链接</a>。</p><h2>小标题</h2><ul><li>第一条</li><li>第二条</li></ul>',
    "text/plain": "网页粗体与斜体，链接。\n小标题\n第一条\n第二条"});
  expect(editor.prosemirrorState.doc.textContent).toContain("网页粗体与斜体，链接。");
  expect(editor.document.map(block => block.type)).toEqual(["paragraph", "heading", "bulletListItem", "bulletListItem"]);
  expect(editor.document[0]?.content).toMatchObject([
    {text: "网页"}, {text: "粗体", styles: {bold: true}}, {text: "与"},
    {text: "斜体", styles: {italic: true}}, {text: "，"}, {type: "link", href: "https://example.org/paper"}, {text: "。"},
  ]);
});

it.each(["notes", "writing"] as const)("prefers browser HTML over Markdown-looking plain text in %s", async kind => {
  const {editor, paste} = await fixture(kind);
  const htmlPaste = vi.spyOn(editor, "pasteHTML");
  const markdownPaste = vi.spyOn(editor, "pasteMarkdown");
  await paste({"text/html": '<p><strong>网页粗体</strong>和字面符号 **原文**</p>', "text/plain": "网页粗体和字面符号 **原文**"});
  expect(htmlPaste).toHaveBeenCalledWith('<p><strong>网页粗体</strong>和字面符号 **原文**</p>');
  expect(markdownPaste).not.toHaveBeenCalled();
  expect(editor.document[0]?.content).toEqual(expect.arrayContaining([{type: "text", text: "网页粗体", styles: {bold: true}}]));
});

it.each(["notes", "writing"] as const)("accepts browser CSS colors at the %s persistence boundary", async kind => {
  const {editor, paste} = await fixture(kind);
  await paste({"text/html": '<meta charset="utf-8"><span style="color: rgb(32, 33, 36); background-color: rgb(255, 255, 255); font-family: Arial; font-size: 14px">网页文字<strong>保留粗体</strong></span>',
    "text/plain": "网页文字保留粗体"});
  expect(editor.prosemirrorState.doc.textContent).toBe("网页文字保留粗体");
  expect(() => blockNoteToNoteDocument(notePageId("paste-fixture"), editor.document as NoteEditorBlock[])).not.toThrow();
  expect(() => decodeWritingNote(JSON.stringify({schemaVersion: 1, noteId: "paste-fixture", blocks: editorToBlocks(editor.document)}))).not.toThrow();
  expect(editor.document[0]?.content).toEqual([
    {type: "text", text: "网页文字", styles: {}}, {type: "text", text: "保留粗体", styles: {bold: true}},
  ]);
});

it.each(["notes", "writing"] as const)("keeps one native replacement/undo operation for colored browser text in %s", async kind => {
  const {editor, paste} = await fixture(kind, "原正文");
  await act(async () => {
    const {$from} = editor.prosemirrorState.selection;
    editor.prosemirrorView.dispatch(closeHistory(editor.prosemirrorState.tr)
      .setSelection(TextSelection.create(editor.prosemirrorState.doc, $from.start(), $from.end())));
  });
  const before = editor.prosemirrorState.doc;
  await paste({"text/html": '<p><span style="color: #223344"><strong>替换文字</strong>与<code>code()</code></span></p>',
    "text/plain": "替换文字与code()"});
  expect(editor.prosemirrorState.doc.textContent).toBe("替换文字与code()");
  const after = editor.prosemirrorState.doc;
  await act(async () => {expect(editor.undo()).toBe(true);});
  expect(editor.prosemirrorState.doc.eq(before)).toBe(true);
  expect(editor.undo()).toBe(false);
  await act(async () => {expect(editor.redo()).toBe(true);});
  expect(editor.prosemirrorState.doc.eq(after)).toBe(true);
});

it.each(["blocknote/html", "text/html"])("retains internal palette colors, empty blocks and nesting through %s", async format => {
  const {editor, paste} = await fixture("notes");
  const html = await editor.blocksToFullHTML([
    {id: "source", type: "paragraph", content: [{type: "text", text: "蓝色粗体", styles: {bold: true, textColor: "blue", backgroundColor: "yellow"}}],
      children: [{id: "source-child", type: "paragraph", content: "嵌套正文"}]},
    {id: "source-empty", type: "paragraph", content: ""},
    {id: "source-last", type: "paragraph", content: "最后一段"},
  ]);
  await paste({[format]: html, "text/plain": "蓝色粗体\n\n嵌套正文\n\n最后一段"});
  expect(editor.document).toHaveLength(3);
  expect(editor.document[0]?.content).toMatchObject([{text: "蓝色粗体", styles: {bold: true, textColor: "blue", backgroundColor: "yellow"}}]);
  expect(editor.document[0]?.children[0]?.content).toMatchObject([{text: "嵌套正文"}]);
  expect(editor.document[1]?.content).toEqual([]);
  expect(editor.document[2]?.content).toMatchObject([{text: "最后一段"}]);
  expect(() => blockNoteToNoteDocument(notePageId("paste-fixture"), editor.document as NoteEditorBlock[])).not.toThrow();
});

it("normalizes browser colors inside table cells and links without flattening their structure", async () => {
  const {editor, paste} = await fixture("notes");
  await paste({"text/html": '<table><tr><td><span style="color: rgb(1, 2, 3)"><a href="https://example.org/paper"><em>证据</em></a></span></td><td><span style="background-color: #fafafa">结果</span></td></tr></table>',
    "text/plain": "证据\t结果"});
  expect(editor.document[0]?.type).toBe("table");
  const domain = blockNoteToNoteDocument(notePageId("paste-fixture"), editor.document as NoteEditorBlock[]);
  expect(domain.blocks[0]?.content).toMatchObject({kind: "table", rows: [[
    {items: [{type: "link", href: "https://example.org/paper", content: [{text: "证据", styles: ["italic"]}]}]},
    {items: [{type: "text", text: "结果", styles: []}]},
  ]]});
});

it("normalizes browser colors on paragraph and heading attributes", async () => {
  const {editor, paste} = await fixture("notes");
  await paste({"text/html": '<h2 style="color: rgb(12, 34, 56); background-color: #fafafa">网页标题</h2><p style="color: #223344; background-color: #eeeeee">正文<strong>粗体</strong></p>', "text/plain": "网页标题\n正文粗体"});
  expect(editor.document.map(block => block.type)).toEqual(["heading", "paragraph"]);
  for (const block of editor.document) expect(block.props).toMatchObject({textColor: "default", backgroundColor: "default"});
  expect(() => blockNoteToNoteDocument(notePageId("paste-fixture"), editor.document as NoteEditorBlock[])).not.toThrow();
});

it("keeps existing plain text, Markdown and read-only paste behavior", async () => {
  const {editor, paste} = await fixture("notes");
  await paste({"text/plain": "第一行\n第二行"});
  expect(editor.prosemirrorState.doc.textContent).toContain("第一行");
  expect(editor.prosemirrorState.doc.textContent).toContain("第二行");
  await act(async () => editor.setTextCursorPosition(editor.document.at(-1)!.id, "end"));
  await paste({"text/markdown": "\n\n**Markdown 粗体**"});
  expect(JSON.stringify(editor.document)).toContain('"bold":true');
  const readonly = await fixture("writing", "只读正文", true);
  const before = readonly.editor.prosemirrorState.doc;
  await readonly.paste({"text/html": '<p><span style="color:#123456">不应插入</span></p>', "text/plain": "不应插入"});
  expect(readonly.editor.prosemirrorState.doc.eq(before)).toBe(true);
});
