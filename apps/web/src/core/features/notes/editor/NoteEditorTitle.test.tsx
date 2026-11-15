// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {HbsSession, createNoteDocumentAdapter} from "@research-assistant/core/hbs";
import {notePageId, noteBlockId, type NoteDocumentV1} from "@research-assistant/core/notes";
import {NotesProvider} from "../NotesClient";
import {noteRendererGateway} from "../../../../app/previews/hbs-adapters/noteGateway";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {HbsNoteEditor} from "../../../components/hbs/HbsNoteEditor";
import {documentTitleId, documentTitleText} from "../../../components/block-editor/documentTitle";
import {replaceBlockText} from "../../../components/block-editor/blockTextEditing";
import type {NoteEditorInstance} from "./noteEditorSchema";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const dispose of cleanups.splice(0)) await dispose(); vi.unstubAllGlobals();});
async function setup() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const initial: NoteDocumentV1 = {schemaVersion: 1, pageId: notePageId("title-note"), title: "Original title", blocks: [{
    id: noteBlockId("body"), type: "paragraph", props: {}, content: {kind: "inline", items: [{type: "text", text: "Original body", styles: []}]}, children: [],
  }]};
  let stored = {version: 1, saved: initial, baseline: initial};
  const session = await HbsSession.open({adapter: createNoteDocumentAdapter(directTextDiff), requestId: n => `title-${n}`,
    preservation: {load: () => undefined, save: () => {}}, port: {read: async () => stored, commit: async request => {
      stored = {version: stored.version + 1, saved: request.saved, baseline: request.baseline}; return stored;
    }},
  });
  const commit = vi.fn(), host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let editor: NoteEditorInstance | undefined;
  const render = async (title = "Original title", readOnly = false) => {
    await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
      <HbsNoteEditor session={session} pageTitle={{value: title}} isReadOnly={readOnly} onEditorReady={value => {editor = value;}} />
    </NotesProvider>));
    await act(async () => {await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));});
  };
  cleanups.push(async () => {await act(async () => root.unmount()); session.dispose(); host.remove();});
  await render();
  return {session, initial, host, commit, render, get stored() {return stored;}, get editor() {return editor!;},
    async external(value: NoteDocumentV1) {
      stored = {...stored, version: stored.version + 1, saved: value};
      await act(async () => session.syncLatest());
      await act(async () => {await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));});
    }};
}
it("saves title input in the same HBS document and native history as body input", async () => {
  const f = await setup(), titleId = documentTitleId(f.initial.pageId);
  await act(async () => {f.editor.updateBlock(titleId, {content: "Changed title"});});
  expect(f.session.getSnapshot().content.title).toBe("Changed title");
  expect(f.session.getSnapshot().baselineProjection.title).toBe("Changed title");
  expect(f.commit).not.toHaveBeenCalled();
  await act(async () => f.session.save()); expect(f.stored.saved.title).toBe("Changed title");
  await act(async () => {f.editor.undo();}); expect(f.session.getSnapshot().content).toEqual(f.initial);
});
async function press(editor: NoteEditorInstance, key: string) {
  await act(async () => editor.domElement!.dispatchEvent(new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true})));
}
it("joins title/body as one HBS edit, saves both and undoes/redoes both", async () => {
  const f = await setup();
  await act(async () => f.editor.setTextCursorPosition("body", "start")); await press(f.editor, "Backspace");
  expect(f.editor.document).toHaveLength(1);
  const joined = {...f.initial, title: "Original titleOriginal body", blocks: []};
  expect(f.session.getSnapshot().content).toEqual(joined);
  expect(f.session.getSnapshot().baselineProjection).toEqual(joined);
  await act(async () => f.session.save()); expect(f.stored.saved).toEqual(joined);
  await act(async () => {expect(f.editor.undo()).toBe(true);});
  expect(f.session.getSnapshot().content).toEqual(f.initial); expect(f.editor.document[1]?.id).toBe("body");
  await act(async () => {expect(f.editor.redo()).toBe(true);}); expect(f.session.getSnapshot().content).toEqual(joined);
});
it("moves a pending body candidate into the title without accepting it, including save and undo", async () => {
  const f = await setup();
  const incoming = {...f.initial, blocks: f.initial.blocks.map(block => ({...block,
    content: {kind: "inline" as const, items: [{type: "text" as const, text: "Incoming body", styles: []}]},
  }))};
  await f.external(incoming);
  await act(async () => f.editor.setTextCursorPosition("body", "start")); await press(f.editor, "Backspace");
  expect(f.session.getSnapshot().content).toEqual({...incoming, title: "Original titleIncoming body", blocks: []});
  expect(f.session.getSnapshot().baselineProjection).toEqual({...f.initial, title: "Original titleOriginal body", blocks: []});
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.host.querySelector("[data-hbs-review-decision]")).not.toBeNull();
  await act(async () => f.session.save()); expect(f.stored.baseline.title).toBe("Original titleOriginal body");
  await act(async () => {f.editor.undo();});
  expect(f.session.getSnapshot().content).toEqual(incoming); expect(f.session.getSnapshot().baselineProjection).toEqual(f.initial);
});
it("splits a title with the ordinary text command and restores both sides through undo", async () => {
  const f = await setup();
  await act(async () => f.editor.setTextCursorPosition(documentTitleId(f.initial.pageId), "start"));
  await press(f.editor, "Enter");
  expect(f.editor.document[1]?.content).toEqual([{type: "text", text: "Original title", styles: {}}]);
  expect(f.session.getSnapshot().content.title).toBe("");
  expect(f.session.getSnapshot().baselineProjection).toEqual(f.session.getSnapshot().content);
  await act(async () => {f.editor.undo();}); expect(f.session.getSnapshot().content).toEqual(f.initial);
});
it("installs external title/body changes into the retained native view", async () => {
  const f = await setup(), native = f.editor.prosemirrorView;
  const incoming = {...f.initial, title: "External title"}; await f.external(incoming);
  expect(f.editor.prosemirrorView).toBe(native); expect(documentTitleText(native.state.doc)).toBe("External title");
  expect(f.session.getSnapshot().baselineProjection.title).toBe("Original title");
  await f.render("Earlier metadata receipt"); expect(documentTitleText(native.state.doc)).toBe("External title");
});
it("installs an external empty title without treating it as an omitted text update", async () => {
  const f = await setup(); await f.external({...f.initial,title:""});
  expect(documentTitleText(f.editor.prosemirrorState.doc)).toBe("");
  expect(f.session.getSnapshot().content.title).toBe("");
  expect(f.session.getSnapshot().message).not.toContain("MODEL_EDITOR_INSTALL_FAILED");
});
it("preserves a genuinely empty title through save, Enter, Backspace and undo", async () => {
  const f = await setup(), id = documentTitleId(f.initial.pageId);
  await act(async () => {f.editor.transact(tr => {expect(replaceBlockText(tr,id,"")).toBe(true);});});
  expect(f.session.getSnapshot().content.title).toBe("");
  await act(async () => {await f.session.save();});
  expect(f.stored.saved.title).toBe("");
  await act(async () => {f.editor.setTextCursorPosition(id,"end");}); await press(f.editor,"Enter");
  expect(f.session.getSnapshot().baselineProjection).toEqual(f.session.getSnapshot().content);
  expect(f.editor.document[1]?.content).toEqual([]);
  await press(f.editor,"Backspace");
  expect(f.editor.document).toHaveLength(2); expect(f.session.getSnapshot().content.title).toBe("");
  await act(async () => {f.editor.setTextCursorPosition("body","start");}); await press(f.editor,"Backspace");
  expect(f.session.getSnapshot().content.title).toBe("Original body");
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});
