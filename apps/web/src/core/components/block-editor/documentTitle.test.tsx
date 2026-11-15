// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {AllSelection} from "@tiptap/pm/state";
import {afterEach, expect, it, vi} from "vitest";
import {noteBlockId, notePageId, type NoteDocumentV1} from "@research-assistant/core/notes";
import {BlockEditor} from "./BlockEditor";
import {documentTitleBlock, documentTitleExtension, documentTextEditing, documentTitleId, documentTitleText} from "./documentTitle";
import {canMoveBlocks, moveBlocks} from "./moveBlocks";
import {canOpenSuggestionMenu} from "./suggestionTriggerPolicy";
import {noteEditorSchema, type NoteEditorInstance} from "../../features/notes/editor/noteEditorSchema";
import {blockNoteToNoteDocument} from "../../features/notes/editor/blockNoteAdapter";
import {createNoteEditorInstallPort} from "../../features/notes/editor/noteEditorInstallPort";
import {NoteEditor} from "../../features/notes/editor/NoteEditor";
import {NotesProvider} from "../../features/notes/NotesClient";
import {noteRendererGateway} from "../../../app/previews/hbs-adapters/noteGateway";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const dispose of cleanups.splice(0)) await dispose(); vi.restoreAllMocks(); vi.unstubAllGlobals();});

it("opens a real NoteEditor with pageTitle and publishes its native title without leaking it into body blocks", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const document: NoteDocumentV1 = {schemaVersion: 1, pageId: notePageId("native-title-smoke"), blocks: [{
    id: noteBlockId("body"), type: "paragraph", props: {}, content: {kind: "inline", items: []}, children: [],
  }]};
  const host = globalThis.document.createElement("div"); globalThis.document.body.append(host); const root = createRoot(host);
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  const changed = vi.fn(), imported = vi.fn(); let editor: NoteEditorInstance | undefined;
  const gateway = new Proxy(noteRendererGateway, {get(target, key) {return key === "importNoteAsset" ? imported : Reflect.get(target, key);}});
  await act(async () => root.render(<NotesProvider gateway={gateway}><NoteEditor document={document}
    pageTitle={{value: "真实页面标题"}} onChange={changed} onEditorReady={value => {editor = value;}}/>
  </NotesProvider>));
  if (!editor) throw new Error("真实笔记编辑器未挂载");
  const native = editor;
  expect(host.querySelectorAll(".bn-editor")).toHaveLength(1);
  expect(host.querySelector(".document-title")?.textContent).toBe("真实页面标题");
  expect(blockNoteToNoteDocument(document.pageId, native.document).title).toBe("真实页面标题");
  await act(async () => native.updateBlock(documentTitleId(document.pageId), {content: "编辑后的原生标题"}));
  await vi.waitFor(() => expect(changed).toHaveBeenCalled());
  const saved = changed.mock.lastCall?.[0] as NoteDocumentV1;
  expect(saved.title).toBe("编辑后的原生标题"); expect(saved.blocks.map(block => block.id)).toEqual(["body"]);
  const title = host.querySelector<HTMLElement>(".document-title")!;
  expect(title.dataset.restrictedBlockText).toBe("true");
  const file = new File([Uint8Array.of(1)], "screenshot.png", {type: "image/png"});
  const paste = new Event("paste", {bubbles: true, cancelable: true});
  Object.defineProperty(paste, "clipboardData", {value: {types: ["Files"], files: [file], items: [{kind: "file", type: "image/png", getAsFile: () => file}], getData: () => ""}});
  await act(async () => title.dispatchEvent(paste));
  expect(imported).not.toHaveBeenCalled();
  expect(native.document.filter(block => block.type === "managedImage")).toHaveLength(0);
});
async function setup(body = "Existing body") {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const commit = vi.fn();
  const editor = BlockNoteEditor.create({schema: noteEditorSchema,
    initialContent: [documentTitleBlock("page", "Page title"), {id: "body", type: "paragraph", content: body}],
    extensions: [documentTextEditing(), documentTitleExtension("page")],
  }) as unknown as NoteEditorInstance;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(<BlockEditor editor={editor} onChange={() => {}} />));
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  const press = async (key: string, composing = false) => {
    const event = new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true, isComposing: composing});
    await act(async () => editor.domElement!.dispatchEvent(event)); return event;
  };
  const at = async (id: string, position: "start" | "end" = "start") => {await act(async () => editor.setTextCursorPosition(id, position));};
  return {editor, host, commit, press, at};
}
it("uses one editor and splits text at the caret with the shared paragraph command", async () => {
  const {editor, host, at, press, commit} = await setup();
  expect(host.querySelectorAll(".bn-editor")).toHaveLength(1);
  expect(host.querySelector("input[aria-label='笔记标题']")).toBeNull();
  await at(documentTitleId("page")); await press("Enter");
  expect(editor.document.map(block => block.type)).toEqual(["documentTitle", "paragraph", "paragraph"]);
  expect(documentTitleText(editor.prosemirrorState.doc)).toBe("");
  expect(editor.document[1]?.content).toEqual([{type: "text", text: "Page title", styles: {}}]);
  expect(editor.getTextCursorPosition().block.id).toBe(editor.document[1]?.id);
  expect(editor.document[2]?.id).toBe("body"); expect(commit).not.toHaveBeenCalled();
});
it("reuses an empty first paragraph and ignores IME confirmation Enter", async () => {
  const {editor, at, press} = await setup("");
  await at(documentTitleId("page"), "end"); await press("Enter", true);
  expect(editor.getTextCursorPosition().block.id).toBe(documentTitleId("page"));
  await press("Enter"); expect(editor.document).toHaveLength(3);
  expect(editor.document[1]?.content).toEqual([]);
});
it.each(["", "Existing body"])("joins the first body into the title, including an empty row (%s)", async body => {
  const {editor, at, press} = await setup(body); await at("body"); await press("Backspace");
  expect(editor.document).toHaveLength(1);
  expect(documentTitleText(editor.prosemirrorState.doc)).toBe("Page title" + body);
  expect(editor.prosemirrorState.selection.$from.parentOffset).toBe("Page title".length);
  await act(async () => {expect(editor.undo()).toBe(true);});
  expect(editor.document[1]?.id).toBe("body"); expect(documentTitleText(editor.prosemirrorState.doc)).toBe("Page title");
});
it("uses the same join from Delete and protects incompatible semantic content", async () => {
  const {editor, at, press} = await setup(); await at(documentTitleId("page"), "end"); await press("Delete");
  expect(editor.document).toHaveLength(1); expect(documentTitleText(editor.prosemirrorState.doc)).toBe("Page titleExisting body");
  await act(async () => {editor.undo(); editor.updateBlock("body", {content: [{type: "link", href: "https://example.com", content: "Linked text"}]});});
  await at("body"); await press("Backspace"); expect(editor.document).toHaveLength(2);
  expect(editor.document[1]?.content).toMatchObject([{type: "link", href: "https://example.com"}]);
});
it("keeps the title fixed and prohibits title moves, nesting and suggestion menus", async () => {
  const {editor, at} = await setup(); await at(documentTitleId("page"));
  expect(canOpenSuggestionMenu(editor.prosemirrorState.tr)).toBe(false);
  expect(canMoveBlocks(editor.document, [documentTitleId("page")], {blockId: "body", placement: "after"})).toBe(false);
  expect(moveBlocks(editor, ["body"], {blockId: documentTitleId("page"), placement: "before"})).toBe(false);
  await act(async () => editor.updateBlock(documentTitleId("page"), {type: "heading", props: {level: 2}}));
  expect(editor.document[0]?.type).toBe("documentTitle");
  expect(documentTitleText(editor.prosemirrorState.doc)).toBe("Page title");
});
it("retains an empty title and body after deleting the full document, with native undo", async () => {
  const {editor} = await setup();
  await act(async () => editor.prosemirrorView.dispatch(editor.prosemirrorState.tr.setSelection(new AllSelection(editor.prosemirrorState.doc)).deleteSelection()));
  expect(editor.document.map(block => block.type)).toEqual(["documentTitle", "paragraph"]);
  expect(documentTitleText(editor.prosemirrorState.doc)).toBe("");
  await act(async () => {editor.undo();});
  expect(documentTitleText(editor.prosemirrorState.doc)).toBe("Page title"); expect(editor.document[1]?.id).toBe("body");
});
it("excludes title metadata from body persistence and preserves it during body installation", async () => {
  const {editor} = await setup();
  const before = blockNoteToNoteDocument(notePageId("page"), editor.document);
  expect(before.blocks.map(block => block.id)).toEqual(["body"]);
  const candidate = {...before, blocks: before.blocks.map(block => ({...block, content: {kind: "inline" as const, items: [{type: "text" as const, text: "Changed body", styles: []}]}}))};
  const port = createNoteEditorInstallPort({editor, pageId: before.pageId, canInstall: () => true});
  await act(async () => {port.prepareStructure(before, candidate)();});
  expect(documentTitleText(editor.prosemirrorState.doc)).toBe("Page title"); expect(port.read().blocks).toEqual(candidate.blocks);
});
it("inserts before a non-text first block and keeps nested first paragraphs intact", async () => {
  const {editor, at, press} = await setup("");
  await act(async () => editor.updateBlock("body", {children: [{id: "child", type: "paragraph", content: "Nested"}]}));
  await at(documentTitleId("page")); await press("Enter");
  expect(editor.document).toHaveLength(3); expect(editor.document[2]?.children[0]?.id).toBe("child");
  await act(async () => editor.removeBlocks([editor.document[1]!.id]));
  await act(async () => editor.updateBlock("body", {type: "divider", children: []}));
  await at(documentTitleId("page")); await press("Enter");
  expect(editor.document.map(block => block.type)).toEqual(["documentTitle", "paragraph", "divider"]);
});
it("rejects excessive input without truncating existing content", async () => {
  const {editor, at} = await setup(); await at(documentTitleId("page"));
  await act(async () => editor.prosemirrorView.dispatch(editor.prosemirrorState.tr.insertText(`Line one\n${"x".repeat(260)}`)));
  const title = editor.prosemirrorState.doc.firstChild!.firstChild!.firstChild!;
  expect(title.textContent).toBe("Page title");
  expect(title.type.allowsMarkType(editor.prosemirrorState.schema.marks.bold!)).toBe(false);
});
