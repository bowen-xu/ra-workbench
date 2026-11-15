// @vitest-environment happy-dom
import {act} from "react";
import {createRoot, type Root} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {baseBlockEditorSchema} from "./baseSchema";
import {NoteEditorSurface} from "./NoteEditorSurface";
import {blockNoteToNoteDocument, noteDocumentToBlockNote} from "../../features/notes/editor/blockNoteAdapter";
import {notePageId} from "@research-assistant/core/notes";

let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => {await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();});

function createEditor() {
  return BlockNoteEditor.create({schema: baseBlockEditorSchema, initialContent: [{
    id: "table", type: "table", props: {caption: ""},
    content: {type: "tableContent", rows: [{cells: ["first", "second"]}]},
  }]}) as unknown as typeof baseBlockEditorSchema.BlockNoteEditor;
}
function button(label: string, container: ParentNode = host): HTMLButtonElement {
  const found = container.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
  if (!found) throw new Error(`Missing ${label}`);
  return found;
}
function alignment(editor: ReturnType<typeof createEditor>): unknown {
  const block = editor.getBlock("table");
  return block?.type === "table" ? block.props.textAlignment : undefined;
}

it("aligns the table shell through native history without replacing its table, cells or resize wrapper", async () => {
  const editor = createEditor();
  await act(async () => root.render(<NoteEditorSurface editor={editor} onChange={() => undefined}/>));
  await vi.waitFor(() => expect(host.querySelector('[aria-label="表格操作"]')).not.toBeNull());
  const table = host.querySelector("table"), cell = table?.querySelector("td"), wrapper = table?.closest(".tableWrapper");
  const toolbar = host.querySelector('[aria-label="表格操作"]');
  expect(toolbar?.closest(".block-media-surface")).toBe(wrapper);
  expect(toolbar?.closest(".tableWrapper-inner")).toBeNull();
  expect(host.querySelector(".note-table-caption-slot .block-caption")).toBeNull();
  await act(async () => button("表格对齐方式").click());
  await vi.waitFor(() => expect(document.querySelector('button[aria-label="右对齐"]')).not.toBeNull());
  const option = button("右对齐", document);
  expect(option.closest('[data-overlay-portal="popup"]')).not.toBeNull();
  await act(async () => option.click());
  expect(alignment(editor)).toBe("right");
  expect(host.querySelector(".note-table-block")?.getAttribute("data-text-alignment")).toBe("right");
  expect(host.querySelector("table")).toBe(table); expect(table?.querySelector("td")).toBe(cell);
  expect(table?.textContent).toBe("firstsecond");
  await act(async () => {expect(editor.undo()).toBe(true);});
  expect(alignment(editor)).toBe("left");
  expect(host.querySelector(".note-table-block")?.getAttribute("data-text-alignment")).toBe("left");
  expect(host.querySelector("table")).toBe(table);
  await act(async () => {expect(editor.redo()).toBe(true);});
  expect(alignment(editor)).toBe("right");
  expect(host.querySelector("table")).toBe(table);
});

it("preserves whole-table alignment in the portable document and exported HTML", async () => {
  const editor = createEditor();
  editor.updateBlock("table", {props: {textAlignment: "center", caption: "results"}});
  const saved = blockNoteToNoteDocument(notePageId("table-controls"), editor.document);
  const reopened = BlockNoteEditor.create({schema: baseBlockEditorSchema, initialContent: noteDocumentToBlockNote(saved)});
  expect(reopened.getBlock("table")?.props).toMatchObject({textAlignment: "center", caption: "results"});
  const html = editor.blocksToFullHTML();
  expect(html).toContain('data-text-alignment="center"');
  expect(html).toContain("results");
  const parsed = await editor.tryParseHTMLToBlocks(html);
  expect(parsed[0]?.props).toMatchObject({textAlignment: "center", caption: "results"});
});

it("removes shared table controls in read-only mode and keeps native table content", async () => {
  const editor = createEditor();
  await act(async () => root.render(<NoteEditorSurface editor={editor} readOnly onChange={() => undefined}/>));
  await vi.waitFor(() => expect(host.querySelector("table")?.textContent).toBe("firstsecond"));
  expect(host.querySelector('[aria-label="表格操作"]')).toBeNull();
  expect(host.querySelector(".note-table-caption-slot .block-caption")).toBeNull();
  expect(host.querySelector("table")?.textContent).toBe("firstsecond");
});
