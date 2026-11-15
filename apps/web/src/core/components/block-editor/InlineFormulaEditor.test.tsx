// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { BlockNoteEditor } from "@blocknote/core";
import { EditorView as SourceEditorView } from "@codemirror/view";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BlockEditor } from "./BlockEditor";
import { baseBlockEditorSchema } from "./baseSchema";

// KaTeX checks standards mode when its module initializes. Match index.html.
vi.hoisted(() => {
  if (document.doctype === null) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  // happy-dom reports BackCompat even after a doctype is inserted.
  Object.defineProperty(document, "compatMode", { configurable: true, value: "CSS1Compat" });
});

let host: HTMLDivElement;
let root: Root;
const ignoreSynchronousSelectionChange = (event: Event) => event.stopImmediatePropagation();
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  host = document.createElement("div"); document.body.append(host);
  root = createRoot(host);
  // Browser selectionchange is queued. happy-dom fires it inside CM's update;
  // this unit suite drives editor selections explicitly, with browser coverage below.
  document.addEventListener("selectionchange", ignoreSynchronousSelectionChange, true);
});
afterEach(async () => {
  await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals();
  document.removeEventListener("selectionchange", ignoreSynchronousSelectionChange, true);
});

function engine(source = "E=mc^2") {
  return BlockNoteEditor.create({ schema: baseBlockEditorSchema, initialContent: [{
    id: "paragraph", type: "paragraph", content: [
      "Before ", { type: "inlineFormula", props: { source } }, " After",
    ],
  }] });
}

it("renders inline math in its paragraph without modifying content or opening a source editor", async () => {
  const editor = engine(); const changed = vi.fn();
  await act(async () => root.render(<BlockEditor editor={editor} onChange={changed} />));
  expect(host.querySelectorAll(".bn-block-content")).toHaveLength(1);
  expect(host.querySelector(".inline-equation-preview .katex")).not.toBeNull();
  expect(host.textContent).toContain("Before"); expect(host.textContent).toContain("After");
  expect(document.querySelector("[aria-label='行内公式 LaTeX 源码']")).toBeNull();
  expect(changed).not.toHaveBeenCalled();
});

it("keeps empty formulas read-only without an automatic edit popover", async () => {
  const editor = engine(""); const changed = vi.fn();
  await act(async () => root.render(<BlockEditor editor={editor} readOnly onChange={changed} />));
  const formula = host.querySelector<HTMLButtonElement>(".inline-equation-preview");
  expect(formula?.disabled).toBe(true);
  await act(async () => formula?.click());
  expect(document.querySelector("[aria-label='编辑行内公式']")).toBeNull();
  expect(changed).not.toHaveBeenCalled();
});

it("exports formula source and inline structure without editor controls or floating portals", async () => {
  const editor = engine();
  let html = "";
  let full = "";
  await act(async () => {
    html = await editor.blocksToHTMLLossy(editor.document);
    full = await editor.blocksToFullHTML(editor.document);
  });
  const container = document.createElement("div"); container.innerHTML = html;
  expect(html).toContain("E=mc^2");
  expect(container.textContent).toContain("Before"); expect(container.textContent).toContain("After");
  expect(container.querySelector("button, input, [role='dialog']")).toBeNull();
  const internal = document.createElement("div"); internal.innerHTML = full;
  expect(internal.querySelector("[data-inline-content-type='inlineFormula']")?.getAttribute("data-source")).toBe("E=mc^2");
  expect((await editor.tryParseHTMLToBlocks(full))[0]?.content).toEqual(editor.document[0]?.content);
});

it("removes an empty equation on completion without deleting its paragraph or adjacent text", async () => {
  const editor = engine(" ");
  await act(async () => root.render(<BlockEditor editor={editor} onChange={() => undefined} />));
  const input = document.querySelector<HTMLTextAreaElement>("[aria-label='行内公式 LaTeX 源码']");
  expect(input).not.toBeNull();
  expect(host.querySelector(".inline-equation-preview")?.textContent).toContain("新公式");
  expect(host.querySelector(".inline-equation-placeholder-symbol msqrt")).not.toBeNull();
  await act(async () => input?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })));
  expect(editor.document).toHaveLength(1);
  expect(editor.document[0]?.id).toBe("paragraph");
  expect(editor.document[0]?.content).toEqual([{ type: "text", text: "Before  After", styles: {} }]);
  expect(host.querySelector(".inline-equation-preview")).toBeNull();
  await act(async () => { expect(editor.undo()).toBe(true); });
  expect(host.querySelector(".inline-equation-preview")?.textContent).toContain("新公式");
});

for (const side of ["left", "right"] as const) {
  it(`enters the specific inline equation from the ${side} and exits to the same document boundary`, async () => {
    const { TextSelection } = await import("@tiptap/pm/state");
    const editor = engine();
    await act(async () => root.render(<BlockEditor editor={editor} onChange={() => undefined} />));
    let position = 0;
    editor.prosemirrorState.doc.descendants((node, pos) => { if (node.type.name === "inlineFormula") position = pos; });
    const boundary = position + (side === "right" ? 1 : 0);
    await act(async () => editor.transact(tr => tr.setSelection(TextSelection.create(tr.doc, boundary))));
    const key = side === "left" ? "ArrowRight" : "ArrowLeft";
    const modified = new KeyboardEvent("keydown", { key, shiftKey: true, bubbles: true, cancelable: true });
    await act(async () => editor.prosemirrorView.dom.dispatchEvent(modified));
    expect(document.querySelector("[aria-label='行内公式 LaTeX 源码']")).toBeNull();
    await act(async () => editor.transact(tr => tr.setSelection(TextSelection.create(tr.doc, boundary))));
    const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
    await act(async () => editor.prosemirrorView.dom.dispatchEvent(event));
    expect(event.defaultPrevented).toBe(true);
    const input = document.querySelector<HTMLDivElement>("[aria-label='行内公式 LaTeX 源码']")!;
    expect(input).not.toBeNull();
    // Browser tests verify native range collapse and automatic full selection.
    const sourceView = SourceEditorView.findFromDOM(input)!;
    sourceView.dispatch({ selection: { anchor: side === "left" ? 0 : sourceView.state.doc.length } });
    await act(async () => input.dispatchEvent(new KeyboardEvent("keydown", {
      key: side === "left" ? "ArrowLeft" : "ArrowRight", bubbles: true, cancelable: true,
    })));
    expect(document.querySelector("[aria-label='行内公式 LaTeX 源码']")).toBeNull();
    expect(editor.prosemirrorState.selection.from).toBe(boundary);
    expect(editor.document[0]?.content).toEqual(engine().document[0]?.content);
  });
}
