// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {TextSelection} from "@tiptap/pm/state";
import {afterEach, expect, it, vi} from "vitest";
import {BlockEditor} from "./BlockEditor";
import {blockTextEditing} from "./blockTextEditing";
import {noteEditorSchema, type NoteEditorInstance} from "../../features/notes/editor/noteEditorSchema";
vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const dispose of cleanups.splice(0)) await dispose(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
async function setup(right = "right") {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const editor = BlockNoteEditor.create({schema: noteEditorSchema,
    initialContent: [{id: "a", type: "paragraph", content: "left"}, {id: "b", type: "paragraph", content: right}],
    extensions: [blockTextEditing()],
  }) as unknown as NoteEditorInstance;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(<BlockEditor editor={editor} onChange={() => {}}/>));
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  return {editor, press: async (key: string) => {await act(async () => editor.domElement!.dispatchEvent(new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true})));}};
}
it.each(["", "right"])("ordinary paragraphs use the same empty/nonempty join and undo (%s)", async right => {
  const {editor, press} = await setup(right);
  await act(async () => editor.setTextCursorPosition("b", "start")); await press("Backspace");
  expect(editor.document).toHaveLength(1); expect(editor.document[0]?.content).toMatchObject([{text: "left" + right}]);
  expect(editor.prosemirrorState.selection.$from.parentOffset).toBe(4);
  await act(async () => {editor.undo();}); expect(editor.document.map(block => block.id)).toEqual(["a", "b"]);
});
it("ordinary Delete and beforeinput use the same transform", async () => {
  const {editor} = await setup(); await act(async () => editor.setTextCursorPosition("a", "end"));
  await act(async () => editor.domElement!.dispatchEvent(new InputEvent("beforeinput", {inputType: "deleteContentForward", bubbles: true, cancelable: true})));
  expect(editor.document).toHaveLength(1); expect(editor.document[0]?.content).toMatchObject([{text: "leftright"}]);
});
it("splits a selected text range and retains the original block identity on the left", async () => {
  const {editor, press} = await setup();
  await act(async () => {
    editor.setTextCursorPosition("a", "start");
    const state = editor.prosemirrorState, start = state.selection.from;
    editor.prosemirrorView.dispatch(state.tr.setSelection(TextSelection.create(state.doc, start + 1, start + 3)));
  });
  await press("Enter");
  expect(editor.document[0]).toMatchObject({id: "a", content: [{text: "l"}]});
  expect(editor.document[1]).toMatchObject({type: "paragraph", content: [{text: "t"}]});
  expect(editor.document[2]?.id).toBe("b");
});

it("expires IME paragraph admission and keeps it local to the owning editor", async () => {
  const {editor: first} = await setup(), {editor: second} = await setup();
  let now = 1000;
  vi.spyOn(Date, "now").mockImplementation(() => now);
  await act(async () => {
    first.setTextCursorPosition("a", "end"); second.setTextCursorPosition("a", "end");
    first.domElement!.dispatchEvent(new CompositionEvent("compositionend", {bubbles: true}));
    second.domElement!.dispatchEvent(new InputEvent("beforeinput", {inputType: "insertParagraph", bubbles: true, cancelable: true}));
  });
  expect(first.document).toHaveLength(2);
  expect(second.document).toHaveLength(3);
  now += 501;
  await act(async () => first.domElement!.dispatchEvent(new InputEvent("beforeinput", {inputType: "insertParagraph", bubbles: true, cancelable: true})));
  expect(first.document).toHaveLength(3);
});
