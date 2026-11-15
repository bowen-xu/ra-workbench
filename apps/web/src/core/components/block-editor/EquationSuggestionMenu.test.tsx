// @vitest-environment happy-dom
import {BlockNoteEditor} from "@blocknote/core";
import {SuggestionMenu} from "@blocknote/core/extensions";
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {NoteEditorSurface} from "./NoteEditorSurface";
import {baseBlockEditorSchema} from "./baseSchema";
import {noteEditorSchema} from "../../features/notes/editor/noteEditorSchema";
import {writingNoteSchema} from "../../../../../../plugins/writing_projects/src/features/writing/notes/writingNoteSchema";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable:true, value:"CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const cleanup of cleanups.splice(0)) await cleanup(); vi.unstubAllGlobals();});

it.each(["notes", "writing"] as const)("opens dollar formulas adjoining text and keeps rapid double dollars in one query in %s", async kind => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const editor = BlockNoteEditor.create({schema:kind === "notes" ? noteEditorSchema : writingNoteSchema,
    initialContent:[{id:"body", type:"paragraph", content:"中文"}],
  }) as unknown as typeof baseBlockEditorSchema.BlockNoteEditor;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  await act(async () => root.render(<NoteEditorSurface editor={editor} onChange={() => undefined}/>));
  await act(async () => {editor.setTextCursorPosition("body", "end"); editor.focus();});
  const type = (text: string) => {
    const view = editor.prosemirrorView, {from,to} = view.state.selection;
    if (!view.someProp("handleTextInput", handler => handler(view, from, to, text, () => view.state.tr.insertText(text)))) {
      view.dispatch(view.state.tr.insertText(text));
    }
  };
  const menu = editor.getExtension(SuggestionMenu)!;
  await act(async () => type("$"));
  expect(menu.store.state).toMatchObject({show:true, triggerCharacter:"$", query:""});
  await act(async () => {menu.clearQuery(); menu.closeMenu(); type("$"); type("$");});
  expect(menu.store.state).toMatchObject({show:true, triggerCharacter:"$", query:"$"});
  expect(editor.prosemirrorState.doc.textContent).toBe("中文$$");
});
