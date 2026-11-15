// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { BlockNoteEditor, selectedFragmentToHTML } from "@blocknote/core";
import { TextSelection } from "@tiptap/pm/state";
import type { EditorView } from "@tiptap/pm/view";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { BlockEditor } from "../../../components/block-editor/BlockEditor";
import { useBlockHighlight } from "../../../components/block-editor/BlockHighlight";
import { NoteClipboard } from "./NoteClipboard";
import { noteEditorSchema, type NoteEditorInstance } from "./noteEditorSchema";
import { noteEmptyBlockPresentation } from "./noteEmptyBlockPresentation";
import { noteClipboardText } from "./noteClipboardText";

vi.mock("../NotesClient", () => ({ useNotesClient: () => ({ gateway: {} }), useNotesVersion: () => 0 }));
// Formula rendering is independent of clipboard source fidelity and unavailable
// in happy-dom's quirks-mode document.
vi.mock("katex", () => ({ default: { render: vi.fn() } }));

// Same pinned optional-heading schema compatibility boundary as production.
const createNoteEditor = BlockNoteEditor.create as unknown as (options: Partial<NoteEditorInstance["options"]>) => NoteEditorInstance;
const selectionHTML = selectedFragmentToHTML as unknown as (view: EditorView, editor: NoteEditorInstance) => {
  clipboardHTML: string; externalHTML: string; markdown: string;
};

function clipboardEvent(type: "copy" | "cut" | "paste", values = new Map<string, string>(), supportsCustomMime = true) {
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, "clipboardData", { value: {
    get types() { return [...values.keys()]; },
    getData: (mime: string) => values.get(mime) ?? "",
    setData: (mime: string, value: string) => { if (supportsCustomMime || mime !== "blocknote/html") values.set(mime, value); },
    clearData: () => values.clear(),
  } });
  return { event, values };
}

function GroupSelection() {
  const { select } = useBlockHighlight();
  return <button type="button" onClick={() => select(["first", "empty", "last"])}>选择块</button>;
}

function selectText(editor: NoteEditorInstance, first: string, last: string, fromOffset = 0, toOffset?: number) {
  let from = 0; let to = 0;
  editor.prosemirrorView.state.doc.descendants((node, position) => {
    if (node.attrs.id === first) from = position + 2 + fromOffset;
    if (node.attrs.id === last) to = position + 2 + (toOffset ?? node.firstChild!.content.size);
  });
  editor.prosemirrorView.dispatch(editor.prosemirrorView.state.tr.setSelection(TextSelection.create(editor.prosemirrorView.state.doc, from, to)));
}

describe("note selection clipboard", () => {
  let host: HTMLDivElement; let root: Root; let editor: NoteEditorInstance;
  beforeEach(async () => {
    vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
    host = document.createElement("div"); document.body.append(host); root = createRoot(host);
    editor = createNoteEditor({ schema: noteEditorSchema, extensions: [noteEmptyBlockPresentation()], initialContent: [
      { id: "first", type: "paragraph", content: "First text" },
      { id: "empty", type: "paragraph" },
      { id: "last", type: "paragraph", content: "Last text", children: [{ id: "child", type: "paragraph", content: "Child text" }] },
    ] });
    await act(async () => root.render(<BlockEditor editor={editor} className="note-editor" onChange={() => undefined}>
      <NoteClipboard editor={editor} /><GroupSelection />
    </BlockEditor>));
  });
  afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

  function copy(supportsCustomMime = true) {
    const result = clipboardEvent("copy", new Map(), supportsCustomMime);
    editor.domElement!.dispatchEvent(result.event);
    return result.values;
  }

  it("skips blank blocks in plain text while preserving the upstream block structure and selection", async () => {
    await act(async () => selectText(editor, "first", "last"));
    const expected = selectionHTML(editor.prosemirrorView, editor);
    const selection = editor.prosemirrorView.state.selection.toJSON();
    const original = JSON.stringify(editor.document);
    const values = copy();
    expect(values.get("text/plain")).toBe("First text\n\nLast text");
    expect(values.get("blocknote/html")).toBe(expected.clipboardHTML);
    expect(values.get("text/html")).toBe(expected.clipboardHTML);
    expect(editor.prosemirrorView.state.selection.toJSON()).toEqual(selection);
    expect(JSON.stringify(editor.document)).toBe(original);
    expect(host.querySelectorAll("[data-note-empty-block]")).toHaveLength(1);
    expect(values.get("blocknote/html")).not.toContain("data-note-empty-block");

    await act(async () => {
      editor.setTextCursorPosition("child", "end");
      editor.domElement!.dispatchEvent(clipboardEvent("paste", values).event);
    });
    const child = editor.getBlock("last")!.children;
    expect(child.map((block) => block.type)).toEqual(["paragraph", "paragraph", "paragraph"]);
    expect(JSON.stringify(child)).toContain("First text");
    expect(child[1]!.content).toEqual([]);
  });

  it("respects partial boundary blocks and keeps single-block code/inline copying upstream", async () => {
    await act(async () => selectText(editor, "first", "last", 6, 4));
    expect(copy().get("text/plain")).toBe("text\n\nLast");
    await act(async () => selectText(editor, "first", "first", 0, 5));
    expect(copy().get("text/plain")).toBe(selectionHTML(editor.prosemirrorView, editor).markdown);
    await act(async () => editor.updateBlock("empty", { content: "Filled" }));
    expect(host.querySelectorAll("[data-note-empty-block]")).toHaveLength(0);
    await act(async () => editor.undo());
    expect(host.querySelectorAll("[data-note-empty-block]")).toHaveLength(1);
  });

  it("copies explicitly selected roots with nested children once, retaining blank blocks", async () => {
    await act(async () => [...host.querySelectorAll("button")].find((button) => button.textContent === "选择块")!.click());
    const original = JSON.stringify(editor.document);
    const values = copy();
    expect(values.get("text/plain")).toBe("First text\n\nLast text\n\nChild text");
    const html = document.createElement("div"); html.innerHTML = values.get("blocknote/html")!;
    expect(html.querySelectorAll("[data-content-type='paragraph']")).toHaveLength(4);
    expect(JSON.stringify(editor.document)).toBe(original);
    expect(host.querySelector("style[data-note-block-highlight]")).not.toBeNull();
  });

  it("cuts the same structural selection and retains rich empty blocks and note text formatting", async () => {
    await act(async () => [...host.querySelectorAll("button")].find(button => button.textContent === "选择块")!.click());
    const original = JSON.stringify(editor.document);
    const cut = clipboardEvent("cut");
    await act(async () => editor.domElement!.dispatchEvent(cut.event));
    expect(cut.values.get("text/plain")).toBe("First text\n\nLast text\n\nChild text");
    expect(cut.values.get("text/html")).toBe(cut.values.get("blocknote/html"));
    const html = document.createElement("div"); html.innerHTML = cut.values.get("text/html")!;
    expect(html.querySelectorAll("[data-content-type='paragraph']")).toHaveLength(4);
    expect(editor.document).toHaveLength(1);
    expect(editor.document[0]!.content).toEqual([]);
    await act(async () => editor.undo());
    expect(JSON.stringify(editor.document)).toBe(original);
  });

  it("preserves soft line breaks, code whitespace, and inline formula/reference text", async () => {
    await act(async () => {
      editor.updateBlock("first", { type: "codeBlock", content: "before\n\nafter" });
      editor.updateBlock("last", { content: [
        { type: "text", text: "See ", styles: {} },
        { type: "researchMention", props: { labelSnapshot: "Paper", targetKind: "paper", targetId: "paper" }, content: [{type:"text",text:"@Paper",styles:{}}] },
        { type: "text", text: " and ", styles: {} },
        { type: "inlineFormula", props: { source: "x^2" } },
      ] });
    });
    const doc = editor.prosemirrorView.state.doc;
    expect(noteClipboardText(doc.slice(0, doc.content.size))).toBe("before\n\nafter\n\nSee @Paper and x^2\n\nChild text");
    await act(async () => selectText(editor, "first", "first", 0, 13));
    expect(copy().get("text/plain")).toBe("before\n\nafter");
  });

  it("retains a selection made entirely of blank blocks in the rich clipboard", async () => {
    await act(async () => {
      editor.updateBlock("first", { content: [] });
      editor.updateBlock("last", { content: [], children: [] });
      selectText(editor, "first", "last");
    });
    const values = copy();
    expect(values.has("text/plain")).toBe(true);
    expect(values.get("text/plain")).toBe("");
    const html = document.createElement("div"); html.innerHTML = values.get("blocknote/html")!;
    expect(html.querySelectorAll("[data-content-type='paragraph']")).toHaveLength(3);
    expect(editor.prosemirrorView.state.selection.empty).toBe(false);
  });

  it("preserves blank paragraphs when the system retains only standard HTML and plain text", async () => {
    await act(async () => selectText(editor, "first", "last"));
    const values = copy(false);
    expect(values.has("blocknote/html")).toBe(false);
    expect(values.get("text/plain")).toBe("First text\n\nLast text");
    await act(async () => {
      editor.setTextCursorPosition("child", "end");
      editor.domElement!.dispatchEvent(clipboardEvent("paste", values).event);
    });
    const children = editor.getBlock("last")!.children;
    expect(children).toHaveLength(3);
    expect(children[1]!.content).toEqual([]);
    expect(JSON.stringify(children[2]!.content)).toContain("Last text");
  });
});
