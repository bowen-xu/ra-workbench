// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {TextSelection} from "@tiptap/pm/state";
import {HbsSession, createNoteDocumentAdapter} from "@research-assistant/core/hbs";
import {notePageId, noteBlockId, type NoteDocumentV1} from "@research-assistant/core/notes";
import {NotesProvider} from "../NotesClient";
import {noteRendererGateway} from "../../../../app/previews/hbs-adapters/noteGateway";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {HbsNoteEditor} from "../../../components/hbs/HbsNoteEditor";
import {captionBlockId} from "../../../components/block-editor/captionBlockProjection";
import type {NoteEditorInstance} from "./noteEditorSchema";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const dispose of cleanups.splice(0)) await dispose(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
const caption = (doc: NoteDocumentV1): string => {
  const block = doc.blocks[0]!;
  return block.content.kind === "asset" ? block.content.caption : String(block.props.caption);
};

for (const type of ["image", "table"] as const) it(`keeps a pending ${type} caption pending across native split, save and undo`, async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:caption-hbs");
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
  const initial: NoteDocumentV1 = {schemaVersion: 1, pageId: notePageId("caption-hbs"), blocks: [{
    id: noteBlockId("media"), type, props: type === "table" ? {caption: "AB"} : {}, children: [],
    content: type === "image" ? {kind: "asset", assetId: "a".repeat(64), displayName: "internal.png", mimeType: "image/png", caption: "AB"}
      : {kind: "table", rows: [[{kind: "inline", items: [{type: "text", text: "cell", styles: []}]}]]},
  }]};
  let stored = {version: 1, saved: initial, baseline: initial};
  const session = await HbsSession.open({adapter: createNoteDocumentAdapter(directTextDiff), requestId: n => `caption-${n}`,
    preservation: {load: () => undefined, save: () => {}}, port: {read: async () => stored, commit: async request => {
      stored = {version: stored.version + 1, saved: request.saved, baseline: request.baseline}; return stored;
    }},
  });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let editor: NoteEditorInstance | undefined;
  cleanups.push(async () => {await act(async () => root.unmount()); session.dispose(); host.remove();});
  const gateway = new Proxy(noteRendererGateway, {get(target, name) {return name === "readNoteAsset" ? async () => Uint8Array.of(1) : Reflect.get(target, name);}});
  await act(async () => root.render(<NotesProvider gateway={gateway}>
    <HbsNoteEditor session={session} onEditorReady={value => {editor = value;}}/>
  </NotesProvider>));
  const incoming: NoteDocumentV1 = {...initial, blocks: initial.blocks.map(block => block.content.kind === "asset"
    ? {...block, content: {...block.content, caption: "AXB"}} : {...block, props: {...block.props, caption: "AXB"}})};
  stored = {...stored, version: 2, saved: incoming};
  await act(async () => session.syncLatest());
  await act(async () => {await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));});
  const native = editor!;
  await act(async () => {native.setTextCursorPosition(captionBlockId("media"), "start");});
  const view = native.prosemirrorView;
  await act(async () => view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, view.state.selection.from + 1))));
  await act(async () => native.domElement!.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", bubbles: true, cancelable: true})));
  const result = session.getSnapshot();
  expect(caption(result.content)).toBe("A"); expect(caption(result.baselineProjection)).toBe("A");
  expect(result.content.blocks[1]?.content).toMatchObject({kind: "inline", items: [{text: "XB"}]});
  expect(result.baselineProjection.blocks[1]?.content).toMatchObject({kind: "inline", items: [{text: "B"}]});
  expect(result.content.blocks[0]?.children).toEqual([]);
  await act(async () => session.save());
  expect(stored.baseline).toEqual(result.baselineProjection); expect(stored.saved).toEqual(result.content);
  await act(async () => {native.undo();});
  expect(session.getSnapshot().content).toEqual(incoming); expect(session.getSnapshot().baselineProjection).toEqual(initial);
  await act(async () => {native.redo();}); expect(session.getSnapshot().content).toEqual(result.content);
});
