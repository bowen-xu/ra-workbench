// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {BlockNoteEditor} from "@blocknote/core";
import {baseBlockEditorSchema} from "./baseSchema";
import {NoteEditorSurface} from "./NoteEditorSurface";
import {NoteBlockActions} from "./NoteBlockActions";
import {notify} from "../../services/notifications";
import {ReorderDragHandle} from "../ReorderableListRow";

vi.mock("../../services/notifications", () => ({notify: {success: vi.fn(), error: vi.fn()}}));
vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const clean of cleanups.splice(0)) await clean(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
async function click(label: string) {
  const button = [...document.querySelectorAll<HTMLElement>('button,[role="menuitem"],[role="menuitemradio"]')].find(element => element.getAttribute("aria-label") === label || element.textContent === label);
  expect(button, label).toBeDefined();
  await act(async () => button!.click());
}
async function setup(sourcePath: readonly string[] = ["notes", "note / id"]) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const dispose = async () => {if (!host.isConnected) return; await act(async () => root.unmount()); host.remove();};
  cleanups.push(dispose);
  const editor = BlockNoteEditor.create({schema: baseBlockEditorSchema, initialContent: [
    {id: "target", type: "paragraph", content: [{type: "text", text: "Idea", styles: {bold: true}}], children: [{id: "child", type: "paragraph", content: "Child"}]},
    {id: "caret", type: "paragraph", content: "Other"},
  ]}) as unknown as typeof baseBlockEditorSchema.BlockNoteEditor;
  const changed = vi.fn(), copied = vi.fn(async (_text: string) => {});
  const menuOpen = vi.fn();
  vi.spyOn(navigator.clipboard, "writeText").mockImplementation(copied);
  const render = async (id = "target", readOnly = false) => act(async () => root.render(<>
    <NoteEditorSurface editor={editor} onChange={changed} readOnly={readOnly}/>
    <NoteBlockActions editor={editor} blockId={id} sourcePath={sourcePath} trigger={<ReorderDragHandle/>} setMenuOpen={menuOpen}/>
  </>));
  await render(); await act(async () => editor.setTextCursorPosition("caret"));
  await click("拖动块；点击打开块菜单");
  expect(host.querySelector('[aria-label="拖动块；点击打开块菜单"]')?.getAttribute("aria-expanded")).toBe("true");
  return {editor, changed, copied, render, dispose, menuOpen};
}

it.each([{level: 2, label: "二级标题"}, {level: 4, label: "四级标题"}])("converts the handle's block to $label, preserving content, ID, children and native undo/redo", async ({level, label}) => {
  const {editor} = await setup();
  const before = structuredClone(editor.getBlock("target"));
  await click("转换为"); await click(label);
  expect(editor.getBlock("target")).toMatchObject({id: "target", type: "heading", props: {level}, content: before!.content, children: before!.children});
  expect(editor.getBlock("caret")?.type).toBe("paragraph");
  await act(async () => {expect(editor.undo()).toBe(true);});
  expect(editor.getBlock("target")).toEqual(before);
  await act(async () => {expect(editor.redo()).toBe(true);});
  expect(editor.getBlock("target")).toMatchObject({type: "heading", props: {level}});
});

it.each([
  {path: ["notes", "note / id"], href: "source://notes/note%20%2F%20id:#block{target}"},
  {path: ["writings", "project", "notes", "idea.ranote"], href: "source://writings/project/notes/idea.ranote:#block{target}"},
])("copies the injected $path block source link without changing the document", async ({path, href}) => {
  const {editor, copied, changed} = await setup(path); const before = structuredClone(editor.document); changed.mockClear();
  await click("复制块链接");
  expect(copied).toHaveBeenCalledWith(href);
  expect(editor.document).toEqual(before); expect(changed).not.toHaveBeenCalled();
});

it("keeps clipboard errors visible and leaves the note untouched", async () => {
  const {editor, copied} = await setup(); const before = structuredClone(editor.document);
  copied.mockRejectedValueOnce(new Error("Clipboard denied")); await click("复制块链接");
  expect(notify.error).toHaveBeenCalledWith(expect.stringContaining("Clipboard denied"));
  expect(editor.document).toEqual(before);
});

it("does not release a disposed editor's menu when a delayed clipboard write completes", async () => {
  const {copied, dispose, menuOpen} = await setup();
  let complete!: () => void;
  copied.mockImplementationOnce(() => new Promise<void>(resolve => {complete = resolve;}));
  await click("复制块链接"); await dispose(); menuOpen.mockClear();
  await act(async () => complete());
  expect(menuOpen).not.toHaveBeenCalled();
});

it("hides conversion for a table and refuses a command if editing becomes read-only", async () => {
  const {editor, render} = await setup();
  await act(async () => editor.insertBlocks([{id: "table", type: "table", content: {type: "tableContent", rows: [{cells: ["A"]}]}}], "caret", "after"));
  await render("table");
  expect(document.querySelector('[aria-label="转换为"]')).toBeNull();
  await render("target"); await click("转换为");
  await act(async () => {editor.isEditable = false;});
  await click("二级标题");
  expect(editor.getBlock("target")?.type).toBe("paragraph");
});

it("keeps embedded references out of a lossy conversion to plain code", async () => {
  const {editor} = await setup();
  await act(async () => editor.updateBlock("target", {content: [{type: "inlineFormula", props: {source: "x^2"}}]}));
  await click("转换为");
  const code = [...document.querySelectorAll<HTMLButtonElement>("button")].find(button => button.textContent === "代码块");
  expect(code).toBeUndefined();
  expect(editor.getBlock("target")?.type).toBe("paragraph");
});

it("converts toggles in both directions while preserving child identity and undo", async () => {
  const {editor, render} = await setup();
  const before = structuredClone(editor.getBlock("target"));
  await click("转换为"); await click("可折叠列表");
  expect(editor.getBlock("target")).toMatchObject({type: "toggleListItem", children: before!.children});
  await act(async () => {expect(editor.undo()).toBe(true);});
  expect(editor.getBlock("target")).toEqual(before);
  await act(async () => {expect(editor.redo()).toBe(true);});
  await render(); await click("拖动块；点击打开块菜单");
  await click("转换为"); await click("可折叠二级标题");
  expect(editor.getBlock("target")).toMatchObject({type: "heading", props: {level: 2, isToggleable: true}});
  await render(); await click("拖动块；点击打开块菜单");
  await click("转换为"); await click("二级标题");
  expect(editor.getBlock("target")).toMatchObject({type: "heading", props: {level: 2, isToggleable: false}, children: before!.children});
});
