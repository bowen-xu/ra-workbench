// @vitest-environment happy-dom
import { BlockNoteEditor } from "@blocknote/core";
import { TextSelection } from "@tiptap/pm/state";
import {EditorView} from "@tiptap/pm/view";
import { StrictMode, act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { BlockEditor } from "./BlockEditor";

let host: HTMLDivElement, root: Root;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(EditorView.prototype, "coordsAtPos").mockReturnValue({left: 100, right: 110, top: 100, bottom: 120});
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
});
afterEach(async () => { await act(async () => root.unmount()); host.remove(); vi.useRealTimers(); vi.restoreAllMocks(); vi.unstubAllGlobals(); });

const button = (name: string) => document.querySelector<HTMLButtonElement>(`button[aria-label="${name}"]`)!;
async function openColors(): Promise<void> {
  await act(async () => { button("颜色").focus(); button("颜色").click(); });
}
async function chooseColor(name: string): Promise<void> {
  expect(button(name)).not.toBeNull();
  await act(async () => {
    const item = button(name);
    item.dispatchEvent(new PointerEvent("pointerdown", {bubbles: true}));
    item.dispatchEvent(new MouseEvent("mousedown", {bubbles: true}));
    item.focus();
    item.dispatchEvent(new PointerEvent("pointerup", {bubbles: true}));
    item.dispatchEvent(new MouseEvent("mouseup", {bubbles: true}));
    item.click();
  });
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 80)); });
}
async function setup(onChange = () => undefined, backward = false) {
  const editor = BlockNoteEditor.create({ initialContent: [
    {id: "first", type: "paragraph", content: "First paragraph"},
    {id: "target", type: "paragraph", content: "Color this text"},
  ] });
  await act(async () => root.render(<StrictMode><BlockEditor editor={editor} onChange={onChange} /><input aria-label="Other input" /></StrictMode>));
  let from = 0;
  editor.prosemirrorState.doc.descendants((node, pos) => { if (node.type.name === "paragraph" && node.textContent === "Color this text") from = pos + 1; });
  await act(async () => { editor.transact(tr => tr.setSelection(TextSelection.create(tr.doc,
    backward ? from + 5 : from, backward ? from : from + 5))); editor.focus(); });
  return editor;
}

it.each([false, true])("keeps selection and editor focus across successive colors (backward=%s)", async backward => {
  const editor = await setup(undefined, backward);
  const selection = editor.prosemirrorState.selection.toJSON();
  const focusedWithMenuOpen: boolean[] = [];
  const focus = editor.focus.bind(editor);
  vi.spyOn(editor, "focus").mockImplementation(() => {
    focusedWithMenuOpen.push(button("颜色")?.getAttribute("aria-expanded") === "true"); focus();
  });
  for (const color of ["文本颜色 · 红色", "背景颜色 · 黄色"]) {
    await openColors(); await chooseColor(color);
    expect(editor.prosemirrorState.selection.toJSON()).toEqual(selection);
    expect(editor.isFocused()).toBe(true);
  }
  expect(editor.document[1]!.content).toEqual([
    {type: "text", text: "Color", styles: {textColor: "red", backgroundColor: "yellow"}},
    {type: "text", text: " this text", styles: {}},
  ]);
  expect(focusedWithMenuOpen).toEqual([false, false]);
});

it("clears each color independently and can reuse a recent color without losing selection", async () => {
  const editor = await setup(), selection = editor.prosemirrorState.selection.toJSON();
  for (const color of ["文本颜色 · 红色", "背景颜色 · 黄色", "文本颜色 · 自动"]) {
    await openColors(); await chooseColor(color);
  }
  expect(editor.document[1]!.content).toEqual([
    {type: "text", text: "Color", styles: {backgroundColor: "yellow"}},
    {type: "text", text: " this text", styles: {}},
  ]);
  expect(editor.getActiveStyles().textColor).toBeUndefined();
  await openColors(); await chooseColor("最近使用 · 文本颜色 · 红色");
  expect(editor.getActiveStyles()).toMatchObject({textColor: "red", backgroundColor: "yellow"});
  await openColors(); await chooseColor("背景颜色 · 自动");
  expect(editor.getActiveStyles().backgroundColor).toBeUndefined();
  expect(editor.getActiveStyles().textColor).toBe("red");
  expect(editor.prosemirrorState.selection.toJSON()).toEqual(selection);
  expect(editor.isFocused()).toBe(true);
});

it("returns Escape to the menu trigger without applying a style", async () => {
  const editor = await setup(), before = structuredClone(editor.document);
  await openColors();
  const menu = document.querySelector<HTMLElement>('[role="menu"][aria-label="文本与背景颜色"]')!;
  await act(async () => menu.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true})));
  await act(async () => { await new Promise(resolve => setTimeout(resolve, 80)); });
  expect(button("颜色").getAttribute("aria-expanded")).toBe("false");
  expect(document.activeElement).toBe(button("颜色"));
  expect(editor.document).toEqual(before);
});

it("does not reclaim focus from a control opened by the change callback", async () => {
  const editor = await setup(() => { host.querySelector("input")!.focus(); });
  await openColors(); await chooseColor("文本颜色 · 红色");
  expect(document.activeElement).toBe(host.querySelector("input"));
  expect(editor.getActiveStyles().textColor).toBe("red");
});

it("hides during ancestor scrolling and restores the current selection after momentum stops", async () => {
  const editor = await setup();
  expect(button("颜色")).not.toBeNull();
  const selection = editor.prosemirrorState.selection.toJSON();
  vi.useFakeTimers();
  await act(async () => host.dispatchEvent(new Event("scroll")));
  expect(button("颜色")).toBeNull();
  await act(async () => { vi.advanceTimersByTime(100); host.dispatchEvent(new Event("scroll")); });
  await act(async () => vi.advanceTimersByTime(149));
  expect(button("颜色")).toBeNull();
  await act(async () => vi.advanceTimersByTime(1));
  expect(button("颜色")).not.toBeNull();
  expect(editor.prosemirrorState.selection.toJSON()).toEqual(selection);
});

it("ignores scrolling in unrelated panes and portaled controls", async () => {
  await setup();
  const sibling = host.querySelector("input")!;
  await act(async () => sibling.dispatchEvent(new Event("scroll")));
  expect(button("颜色")).not.toBeNull();
  await act(async () => button("颜色").dispatchEvent(new Event("scroll", {bubbles: true})));
  expect(button("颜色")).not.toBeNull();
});

it("does not resurrect a collapsed selection after document scrolling", async () => {
  const editor = await setup();
  vi.useFakeTimers();
  await act(async () => document.dispatchEvent(new Event("scroll")));
  expect(button("颜色")).toBeNull();
  await act(async () => editor.setTextCursorPosition("target", "end"));
  await act(async () => vi.advanceTimersByTime(150));
  expect(button("颜色")).toBeNull();
  expect(editor.prosemirrorState.selection.empty).toBe(true);
});

it("stays hidden outside the editor's clipped pane and resumes only after scrolling back", async () => {
  const editor = await setup();
  host.style.overflowY = "auto";
  vi.spyOn(host, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 50, 600, 400));
  vi.useFakeTimers();
  vi.mocked(editor.prosemirrorView.coordsAtPos).mockReturnValue({left: 100, right: 110, top: 10, bottom: 30});
  await act(async () => host.dispatchEvent(new Event("scroll")));
  await act(async () => vi.advanceTimersByTime(150));
  expect(button("颜色")).toBeNull();
  vi.mocked(editor.prosemirrorView.coordsAtPos).mockReturnValue({left: 100, right: 110, top: 100, bottom: 120});
  await act(async () => host.dispatchEvent(new Event("scroll")));
  expect(button("颜色")).toBeNull();
  await act(async () => vi.advanceTimersByTime(150));
  expect(button("颜色")).not.toBeNull();
});

it("rechecks selection visibility on viewport resize", async () => {
  const editor = await setup();
  vi.mocked(editor.prosemirrorView.coordsAtPos).mockReturnValue({left: 100, right: 110, top: -50, bottom: -30});
  await act(async () => window.dispatchEvent(new Event("resize")));
  expect(button("颜色")).toBeNull();
});

it("dismisses an open child menu during scrolling without restoring it or changing selection", async () => {
  const editor = await setup(), selection = editor.prosemirrorState.selection.toJSON();
  await openColors();
  expect(document.querySelector('[role="menu"][aria-label="文本与背景颜色"]')).not.toBeNull();
  vi.useFakeTimers();
  await act(async () => host.dispatchEvent(new Event("scroll")));
  expect(document.querySelector('[role="menu"][aria-label="文本与背景颜色"]')).toBeNull();
  await act(async () => vi.advanceTimersByTime(150));
  expect(button("颜色").getAttribute("aria-expanded")).toBe("false");
  expect(editor.prosemirrorState.selection.toJSON()).toEqual(selection);
});
