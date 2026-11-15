// @vitest-environment happy-dom
import { act, createRef, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { ApplicationKeyboardPolicy, installApplicationTabPolicy } from "./ApplicationKeyboardPolicy";
import { writingKeymap } from "../../../../../plugins/writing_projects/src/features/writing/editor/writingKeymap";
import { EquationSourceInput, type EquationSourceInputHandle } from "../components/block-editor/EquationSourceInput";
import { BlockNoteEditor } from "@blocknote/core";
import { BlockEditor } from "../components/block-editor/BlockEditor";
import { blockEditorDefaults } from "../components/block-editor/defaults";

const cleanup: (() => void)[] = [];
afterEach(() => { cleanup.splice(0).reverse().forEach(dispose => dispose()); document.body.innerHTML = ""; vi.unstubAllGlobals(); });
function policy() {cleanup.push(installApplicationTabPolicy(window));}
function press(target: HTMLElement, init: KeyboardEventInit = {}) {
  const event = new KeyboardEvent("keydown", {key: "Tab", bubbles: true, cancelable: true, ...init});
  target.dispatchEvent(event); return event;
}

it.each([false, true])("blocks native traversal and library-driven focus from ordinary controls, shift=%s", shiftKey => {
  policy();
  document.body.innerHTML = '<input><button>Action</button><dialog open><input><button>Close</button></dialog><div role="menu"><button>Item</button></div>';
  const move = vi.fn(() => (document.querySelector("button") as HTMLButtonElement).focus());
  for (const element of document.querySelectorAll<HTMLElement>("input,button")) {
    element.addEventListener("keydown", move); element.focus();
    expect(press(element, {shiftKey}).defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(element);
  }
  expect(move).not.toHaveBeenCalled();
});

it("allows only declared directions, does not inherit a panel's declaration, and cancels unused fallback", () => {
  policy(); document.body.innerHTML = '<button data-tab-action="forward">Confirm</button><div data-tab-action="both"><input></div><input data-tab-action="both">';
  const button = document.querySelector("button")!, action = vi.fn((event: Event) => event.preventDefault());
  button.addEventListener("keydown", action);
  expect(press(button).defaultPrevented).toBe(true); expect(action).toHaveBeenCalledOnce();
  expect(press(button, {shiftKey: true}).defaultPrevented).toBe(true); expect(action).toHaveBeenCalledOnce();
  const child = document.querySelector("div input")!, childAction = vi.fn(); child.addEventListener("keydown", childAction);
  press(child as HTMLElement); expect(childAction).not.toHaveBeenCalled();
  const empty = document.querySelectorAll<HTMLInputElement>("input")[1]!;
  expect(press(empty, {shiftKey: true}).defaultPrevented).toBe(true);
});

it("does not run Tab actions during IME and leaves other keys and system chords alone", () => {
  policy(); document.body.innerHTML = '<input data-tab-action="both">'; const input = document.querySelector("input")!;
  const action = vi.fn(); input.addEventListener("keydown", action);
  for (const init of [{isComposing: true}, {keyCode: 229}]) expect(press(input, init).defaultPrevented).toBe(true);
  expect(action).not.toHaveBeenCalled();
  for (const init of [{key: "Enter"}, {key: "Escape"}, {ctrlKey: true}, {metaKey: true}, {altKey: true}]) expect(press(input, init).defaultPrevented).toBe(false);
  expect(action).toHaveBeenCalledTimes(5);
});

it("has one lifecycle in StrictMode and removes the global rule on unmount", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  cleanup.push(() => act(() => root.unmount()));
  await act(async () => root.render(<StrictMode><ApplicationKeyboardPolicy /><input /></StrictMode>));
  const input = host.querySelector("input")!; expect(press(input).defaultPrevented).toBe(true);
  await act(async () => root.render(<input />));
  expect(press(host.querySelector("input")!).defaultPrevented).toBe(false);
});

it("preserves the real writing editor's indent and reverse-indent commands", () => {
  policy(); const host = document.createElement("div"); document.body.append(host);
  const view = new EditorView({parent: host, state: EditorState.create({doc: "x", selection: {anchor: 0}, extensions: writingKeymap({onCompile() {}, onOpenCommands() {}})})});
  cleanup.push(() => view.destroy()); view.focus();
  expect(press(view.contentDOM).defaultPrevented).toBe(true);
  expect(view.state.doc.toString()).toMatch(/^\s+x$/);
  expect(press(view.contentDOM, {shiftKey: true}).defaultPrevented).toBe(true);
  expect(view.state.doc.toString()).toBe("x"); expect(document.activeElement).toBe(view.contentDOM);
});

it("preserves formula Tab insertion but makes Shift+Tab without a snippet a no-op", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); policy();
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  cleanup.push(() => act(() => root.unmount()));
  const ref = createRef<EquationSourceInputHandle>(); const change = vi.fn();
  await act(async () => root.render(<EquationSourceInput inputRef={ref} value="x" onChange={change} onComplete={() => {}} onCancel={() => {}} aria-label="Equation" placeholder="" />));
  ref.current!.focus(); const input = host.querySelector<HTMLElement>(".cm-content")!;
  await act(async () => {expect(press(input, {shiftKey: true}).defaultPrevented).toBe(true);});
  expect(change).not.toHaveBeenCalled(); expect(document.activeElement).toBe(input);
  await act(async () => {expect(press(input).defaultPrevented).toBe(true);});
  expect(change.mock.lastCall?.[0]).toContain("\t"); expect(document.activeElement).toBe(input);
});

it("preserves the real block editor's nesting while blocking Tab on its unrelated toolbar", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); policy();
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  cleanup.push(() => act(() => root.unmount()));
  const editor = BlockNoteEditor.create({...blockEditorDefaults, initialContent: [
    {id: "first", type: "paragraph", content: "First"}, {id: "second", type: "paragraph", content: "Second"},
  ]});
  await act(async () => root.render(<BlockEditor editor={editor} onChange={() => {}} />));
  await act(async () => {editor.setTextCursorPosition("second", "start"); editor.focus();});
  await act(async () => {expect(press(editor.prosemirrorView.dom).defaultPrevented).toBe(true);});
  expect(editor.document[0]!.children[0]?.id).toBe("second");
  await act(async () => {expect(press(editor.prosemirrorView.dom, {shiftKey: true}).defaultPrevented).toBe(true);});
  expect(editor.document.map(block => block.id)).toEqual(["first", "second"]);
});
