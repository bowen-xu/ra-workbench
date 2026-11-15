// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {TextSelection} from "@tiptap/pm/state";
import {afterEach, expect, it, vi} from "vitest";
import {baseBlockEditorSchema} from "./baseSchema";
import {NoteEditorSurface} from "./NoteEditorSurface";
import {BLOCK_TEXT_INTENT} from "./blockTextEditing";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) await cleanup();
  localStorage.removeItem("toggle-parent"); vi.restoreAllMocks(); vi.unstubAllGlobals();
});
type Block = typeof baseBlockEditorSchema.PartialBlock;
async function setup(children: Block[], type: "toggleListItem" | "heading" = "toggleListItem") {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.setItem("toggle-parent", "true");
  const editor = BlockNoteEditor.create({schema: baseBlockEditorSchema, initialContent: [
    {id: "parent", type, props: type === "heading" ? {level: 2, isToggleable: true} : {}, content: "Title", children},
    {id: "after", type: "paragraph", content: "After"},
  ]});
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(<NoteEditorSurface editor={editor} onChange={() => {}}/>));
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  return {editor, host,
    at: async (id: string) => {await act(async () => editor.setTextCursorPosition(id, "start"));},
    key: async (options: KeyboardEventInit = {}) => {
      await act(async () => editor.domElement!.dispatchEvent(new KeyboardEvent("keydown", {key: "Backspace", keyCode: 8, bubbles: true, cancelable: true, ...options})));
    },
  };
}
const empty = {id: "empty", type: "paragraph", content: ""} as const;
const before = {id: "before", type: "paragraph", content: "Before"} as const;
const next = {id: "next", type: "paragraph", content: "Next"} as const;

it.each(["toggleListItem", "heading"] as const)("restores the native empty hint for an only child of %s, with undo/redo", async type => {
  const f = await setup([empty], type); await f.at("empty");
  const original = structuredClone(f.editor.document);
  await f.key();
  expect(f.editor.getBlock("parent")?.children).toEqual([]);
  expect(f.editor.getTextCursorPosition().block.id).toBe("parent");
  expect(f.editor.prosemirrorState.selection.$from.parentOffset).toBe(5);
  expect(f.host.querySelector(".bn-toggle-add-block-button")).not.toBeNull();
  const changed = structuredClone(f.editor.document);
  await act(async () => {expect(f.editor.undo()).toBe(true);});
  expect(f.editor.document).toEqual(original);
  expect(f.editor.getTextCursorPosition().block.id).toBe("empty");
  await act(async () => {expect(f.editor.redo()).toBe(true);});
  expect(f.editor.document).toEqual(changed);
});
it.each([
  {name: "first", children: [empty, next], caret: "parent", offset: 5},
  {name: "middle", children: [before, empty, next], caret: "before", offset: 6},
])("removes an empty $name row and places the caret at the preceding title's end", async ({children, caret, offset}) => {
  const f = await setup(children); await f.at("empty");
  const original = structuredClone(f.editor.document);
  await f.key();
  expect(f.editor.getBlock("parent")?.children.map(block => block.id)).toEqual(children.filter(block => block.id !== "empty").map(block => block.id));
  expect(f.editor.document.map(block => block.id)).toEqual(["parent", "after"]);
  expect(f.editor.getTextCursorPosition().block.id).toBe(caret);
  expect(f.editor.prosemirrorState.selection.$from.parentOffset).toBe(offset);
  const changed = structuredClone(f.editor.document);
  await act(async () => {f.editor.undo();}); expect(f.editor.document).toEqual(original);
  await act(async () => {f.editor.redo();}); expect(f.editor.document).toEqual(changed);
});
it.each([
  {type: "toggleListItem" as const, text: ""}, {type: "toggleListItem" as const, text: "Right"},
  {type: "heading" as const, text: ""}, {type: "heading" as const, text: "Right"},
])("outdents the last row regardless of content ($type, text=$text)", async ({type, text}) => {
  const f = await setup([before, {...empty, content: text}], type); await f.at("empty");
  const original = structuredClone(f.editor.document), row = structuredClone(f.editor.getBlock("empty"));
  await f.key();
  expect(f.editor.getBlock("empty")).toEqual(row);
  expect(f.editor.getBlock("parent")?.children.map(block => block.id)).toEqual(["before"]);
  expect(f.editor.document.map(block => block.id)).toEqual(["parent", "empty", "after"]);
  expect(f.editor.getTextCursorPosition().block.id).toBe("empty");
  await act(async () => {f.editor.undo();}); expect(f.editor.document).toEqual(original);
  await act(async () => {f.editor.redo();}); expect(f.editor.document.map(block => block.id)).toEqual(["parent", "empty", "after"]);
});
it("uses the same rule for WebKit beforeinput deletion", async () => {
  const f = await setup([empty, next]); await f.at("empty");
  const event = new InputEvent("beforeinput", {inputType: "deleteContentBackward", bubbles: true, cancelable: true});
  await act(async () => f.editor.domElement!.dispatchEvent(event));
  expect(event.defaultPrevented).toBe(true);
  expect(f.editor.getBlock("parent")?.children.map(block => block.id)).toEqual(["next"]);
});
it.each([{isComposing: true}, {keyCode: 229}])("does not restructure during IME composition (%j)", async options => {
  const f = await setup([empty, next]); await f.at("empty");
  const original = structuredClone(f.editor.document); await f.key(options);
  expect(f.editor.document).toEqual(original);
});
it("keeps native behavior for nonempty children and never deletes owned descendants", async () => {
  const f = await setup([{...empty, children: [next]}, before]); await f.at("empty");
  await f.key();
  expect(f.editor.getBlock("next")?.content).toMatchObject([{text: "Next"}]);
  expect(f.editor.getBlock("empty")).toBeDefined();
  await f.at("before"); await f.key();
  expect(f.editor.getBlock("before")?.content).toMatchObject([{text: "Before"}]);
});
it("lets a selected range follow native deletion rather than empty-row rules", async () => {
  const f = await setup([before, empty, next]); await f.at("before");
  await act(async () => {
    const state = f.editor.prosemirrorState, from = state.selection.from;
    f.editor.prosemirrorView.dispatch(state.tr.setSelection(TextSelection.create(state.doc, from, from + 6)));
  });
  await f.key();
  expect(f.editor.getBlock("before")?.content).toEqual([]);
  expect(f.editor.getBlock("parent")?.children.map(block => block.id)).toEqual(["before", "empty", "next"]);
});
it("retains ordinary indentation rules outside toggles, including a disabled toggle heading", async () => {
  const f = await setup([empty, next]);
  await act(async () => f.editor.updateBlock("parent", {type: "heading", props: {level: 2, isToggleable: false}}));
  await f.at("empty"); await f.key();
  expect(f.editor.document.map(block => block.id)).toEqual(["parent", "empty", "after"]);
  expect(f.editor.getBlock("empty")?.children.map(block => block.id)).toEqual(["next"]);
});
it("keeps whitespace content and leaves ordinary text deletion to the engine", async () => {
  const f = await setup([{...empty, content: " "}, next]); await f.at("empty"); await f.key();
  expect(f.editor.getBlock("empty")?.content).toMatchObject([{text: " "}]);
});
it("handles a nested toggle's only child without changing the outer tree", async () => {
  const f = await setup([{id: "inner", type: "toggleListItem", content: "Inner", children: [empty]}, next]);
  await f.at("empty"); await f.key();
  expect(f.editor.getBlock("parent")?.children.map(block => block.id)).toEqual(["inner", "next"]);
  expect(f.editor.getBlock("inner")?.children).toEqual([]);
  expect(f.editor.getTextCursorPosition().block.id).toBe("inner");
  expect(f.host.querySelectorAll(".bn-toggle-add-block-button")).toHaveLength(1);
});
it("keeps the structural deletion separate from subsequent typing in undo history", async () => {
  const f = await setup([empty, next]); await f.at("empty"); await f.key();
  const deleted = structuredClone(f.editor.document);
  await act(async () => f.editor.prosemirrorView.dispatch(f.editor.prosemirrorState.tr.insertText("x")));
  await act(async () => {f.editor.undo();}); expect(f.editor.document).toEqual(deleted);
  await act(async () => {f.editor.undo();});
  expect(f.editor.getBlock("parent")?.children.map(block => block.id)).toEqual(["empty", "next"]);
});
it("uses native selection when the preceding sibling is an atomic block", async () => {
  const f = await setup([{id: "equation", type: "formulaBlock", props: {source: "x^2"}}, empty, next]);
  await f.at("empty"); await f.key();
  expect(f.editor.getBlock("parent")?.children.map(block => block.id)).toEqual(["equation", "next"]);
  expect(f.editor.getTextCursorPosition().block.id).toBe("equation");
  expect(f.editor.getBlock("equation")?.props).toMatchObject({source: "x^2"});
});
it.each([
  {type: "toggleListItem" as const, last: false},
  {type: "heading" as const, last: false},
])("joins a nonempty child to its previous sibling without outdenting ($type, last=$last)", async ({type, last}) => {
  const f = await setup([before, {...empty, content: "Right"}, ...last ? [] : [next]], type);
  await f.at("empty"); const original = structuredClone(f.editor.document);
  const dispatch = vi.spyOn(f.editor.prosemirrorView, "dispatch");
  await f.key();
  expect(dispatch.mock.calls.some(([transaction]) => {
    const intent = transaction.getMeta(BLOCK_TEXT_INTENT);
    return intent?.kind === "join" && intent.left === "before" && intent.right === "empty";
  })).toBe(true);
  dispatch.mockRestore();
  expect(f.editor.document.map(block => block.id)).toEqual(["parent", "after"]);
  expect(f.editor.getBlock("parent")?.children.map(block => block.id)).toEqual(last ? ["before"] : ["before", "next"]);
  expect(f.editor.getBlock("before")?.content).toMatchObject([{text: "BeforeRight"}]);
  expect(f.editor.getTextCursorPosition().block.id).toBe("before");
  expect(f.editor.prosemirrorState.selection.$from.parentOffset).toBe(6);
  await act(async () => {f.editor.undo();}); expect(f.editor.document).toEqual(original);
  await act(async () => {f.editor.redo();});
  expect(f.editor.getBlock("before")?.content).toMatchObject([{text: "BeforeRight"}]);
});
it("preserves rich text, links and inline equations when joining a nonempty child", async () => {
  const content = [
    {type: "text", text: "Right", styles: {bold: true}},
    {type: "inlineFormula", props: {source: "x_1"}},
    {type: "link", href: "https://example.org", content: "Link"},
  ] as const;
  const f = await setup([before, {...empty, content: [...content]}, next]);
  await f.at("empty"); const original = structuredClone(f.editor.document); await f.key();
  expect(f.editor.getBlock("before")?.content).toMatchObject([
    {type: "text", text: "Before"}, {type: "text", text: "Right", styles: {bold: true}},
    {type: "inlineFormula", props: {source: "x_1"}}, {type: "link", href: "https://example.org"},
  ]);
  expect(f.editor.getBlock("parent")?.children.map(block => block.id)).toEqual(["before", "next"]);
  await act(async () => {f.editor.undo();}); expect(f.editor.document).toEqual(original);
});
it("handles nonempty backward beforeinput with the same join policy", async () => {
  const f = await setup([before, {...empty, content: "Right"}, next]); await f.at("empty");
  const event = new InputEvent("beforeinput", {inputType: "deleteContentBackward", bubbles: true, cancelable: true});
  await act(async () => f.editor.domElement!.dispatchEvent(event));
  expect(event.defaultPrevented).toBe(true);
  expect(f.editor.getBlock("before")?.content).toMatchObject([{text: "BeforeRight"}]);
  expect(f.editor.document.map(block => block.id)).toEqual(["parent", "after"]);
});
it("does not intercept character deletion inside a nonempty child", async () => {
  const f = await setup([before, {...empty, content: "Right"}, next]);
  await act(async () => f.editor.setTextCursorPosition("empty", "end"));
  const event = new KeyboardEvent("keydown", {key: "Backspace", keyCode: 8, bubbles: true, cancelable: true});
  await act(async () => f.editor.domElement!.dispatchEvent(event));
  expect(event.defaultPrevented).toBe(false);
  expect(f.editor.getBlock("parent")?.children.map(block => block.id)).toEqual(["before", "empty", "next"]);
});
it("outdents a nonempty last row through backward beforeinput", async () => {
  const f = await setup([before, {...empty, content: "Right"}]); await f.at("empty");
  const event = new InputEvent("beforeinput", {inputType: "deleteContentBackward", bubbles: true, cancelable: true});
  await act(async () => f.editor.domElement!.dispatchEvent(event));
  expect(event.defaultPrevented).toBe(true);
  expect(f.editor.document.map(block => block.id)).toEqual(["parent", "empty", "after"]);
  expect(f.editor.getBlock("empty")?.content).toMatchObject([{text: "Right"}]);
});
it("lifts the last row with its rich content and descendants intact", async () => {
  const f = await setup([before, {...empty, content: [{type: "text", text: "Right", styles: {bold: true}}], children: [next]}]);
  await f.at("empty"); const original = structuredClone(f.editor.document), row = structuredClone(f.editor.getBlock("empty"));
  await f.key();
  expect(f.editor.document.map(block => block.id)).toEqual(["parent", "empty", "after"]);
  expect(f.editor.getBlock("empty")).toEqual(row);
  expect(f.editor.getParentBlock("next")?.id).toBe("empty");
  await act(async () => {f.editor.undo();}); expect(f.editor.document).toEqual(original);
});
