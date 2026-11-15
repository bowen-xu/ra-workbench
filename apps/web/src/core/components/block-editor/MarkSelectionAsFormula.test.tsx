// @vitest-environment happy-dom
import { BlockNoteEditor } from "@blocknote/core";
import { TextSelection } from "@tiptap/pm/state";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BlockEditor } from "./BlockEditor";
import { baseBlockEditorSchema } from "./baseSchema";

vi.hoisted(() => Object.defineProperty(document, "compatMode", { configurable: true, value: "CSS1Compat" }));
let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); });

function engine() {
  return BlockNoteEditor.create({ schema: baseBlockEditorSchema, initialContent: [
    { id: "p", type: "paragraph", content: ["Before ", { type: "text", text: "E=mc^2", styles: { bold: true } }, " After"] },
    { id: "tail", type: "paragraph", content: "Tail" },
  ] });
}
type Engine = ReturnType<typeof engine>;
async function select(editor: Engine, text: string) {
  let position = -1;
  editor.prosemirrorState.doc.descendants((node, pos) => { if (node.isText && node.text?.includes(text)) position = pos + node.text.indexOf(text); });
  expect(position).toBeGreaterThanOrEqual(0);
  await act(async () => editor.transact(tr => tr.setSelection(TextSelection.create(tr.doc, position, position + text.length))));
}
async function press(editor: Engine, modifiers: KeyboardEventInit = { metaKey: true, shiftKey: true }) {
  const event = new KeyboardEvent("keydown", { key: "E", bubbles: true, cancelable: true, ...modifiers });
  await act(async () => editor.prosemirrorView.dom.dispatchEvent(event));
  return event;
}

it.each(["metaKey", "ctrlKey"] as const)("converts only selected source with %s, keeps adjacent text and supports undo/redo", async key => {
  const editor = engine(), changed = vi.fn();
  await act(async () => root.render(<BlockEditor editor={editor} onChange={changed} defaultFormattingToolbar={false} />));
  const before = editor.document;
  await select(editor, "E=mc^2");
  expect((await press(editor, { [key]: true, shiftKey: true })).defaultPrevented).toBe(true);
  expect(editor.document[0]?.content).toEqual([
    { type: "text", text: "Before ", styles: {} }, { type: "inlineFormula", props: { source: "E=mc^2" } },
    { type: "text", text: " After", styles: {} },
  ]);
  expect(editor.document[1]).toEqual(before[1]);
  expect(editor.prosemirrorState.selection.empty).toBe(true);
  expect(editor.prosemirrorState.selection.$from.nodeBefore?.type.name).toBe("inlineFormula");
  expect(document.querySelector('[aria-label="编辑行内公式"]')).toBeNull();
  await act(async () => { expect(editor.undo()).toBe(true); });
  expect(editor.document).toEqual(before);
  await act(async () => { expect(editor.redo()).toBe(true); });
  expect(host.querySelectorAll(".inline-equation-preview")).toHaveLength(1);
});

it("does not consume composing, unmodified or readonly keyboard input", async () => {
  const editor = engine();
  await act(async () => root.render(<BlockEditor editor={editor} onChange={() => undefined} />));
  await select(editor, "E=mc^2"); const before = editor.document;
  for (const modifiers of [{}, { metaKey: true }, { metaKey: true, shiftKey: true, isComposing: true }, { metaKey: true, shiftKey: true, altKey: true }]) {
    expect((await press(editor, modifiers)).defaultPrevented).toBe(false);
    expect(editor.document).toEqual(before);
  }
  await act(async () => root.render(<BlockEditor editor={editor} readOnly onChange={() => undefined} />));
  expect((await press(editor)).defaultPrevented).toBe(false); expect(editor.document).toEqual(before);
});

it("rejects empty and cross-block selections without changing document structure", async () => {
  const editor = engine();
  await act(async () => root.render(<BlockEditor editor={editor} onChange={() => undefined} />));
  const before = editor.document;
  await act(async () => editor.setTextCursorPosition("p", "start"));
  expect((await press(editor)).defaultPrevented).toBe(false);
  await act(async () => editor.setSelection("p", "tail"));
  expect((await press(editor)).defaultPrevented).toBe(false); expect(editor.document).toEqual(before);
});

it("does not convert code or existing atomic equations", async () => {
  const editor = engine();
  await act(async () => root.render(<BlockEditor editor={editor} onChange={() => undefined} />));
  await act(async () => editor.updateBlock("p", { content: [{ type: "text", text: "x^2", styles: { code: true } }] }));
  await select(editor, "x^2"); let before = editor.document;
  expect((await press(editor)).defaultPrevented).toBe(false); expect(editor.document).toEqual(before);
  await act(async () => editor.updateBlock("p", { content: "AB" }));
  let position = 0;
  editor.prosemirrorState.doc.descendants((node, pos) => { if (node.type.name === "paragraph" && node.textContent === "AB") position = pos + 1; });
  const formula = editor.pmSchema.nodes.inlineFormula;
  if (!formula) throw new Error("Missing inline formula schema");
  await act(async () => editor.transact(tr => tr.insert(position + 1, formula.create({ source: "x^2" }))));
  await act(async () => editor.transact(tr => tr.setSelection(TextSelection.create(tr.doc, position, position + 3))));
  before = editor.document;
  expect((await press(editor)).defaultPrevented).toBe(false); expect(editor.document).toEqual(before);
});
