// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {HbsSession, createNoteDocumentAdapter} from "@research-assistant/core/hbs";
import {notePageId, noteBlockId, type NoteDocumentV1, type NoteBlockV1} from "@research-assistant/core/notes";
import {NotesGatewayError, type NotesGateway} from "@research-assistant/data/notes";
import {NotesProvider} from "../NotesClient";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {HbsNoteEditor} from "../../../components/hbs/HbsNoteEditor";
import type {NoteEditorInstance} from "./noteEditorSchema";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const paragraph = (id: string, text: string): NoteBlockV1 => ({id: noteBlockId(id), type: "paragraph", props: {},
  content: {kind: "inline", items: [{type: "text", text, styles: []}]}, children: []});
const gateway = new Proxy({
  getReadingNote: async () => {throw new NotesGatewayError("not_found", "This synthetic toggle has no reading source.");},
}, {get(target, key) {
  if (key in target) return Reflect.get(target, key);
  return async () => {throw new Error(`Unexpected note gateway call: ${String(key)}`);};
}}) as unknown as NotesGateway;

it.each([false, true])("keeps pending child text pending across a toggle join/lift, save and undo/redo (last=%s)", async last => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const initial: NoteDocumentV1 = {schemaVersion: 1, pageId: notePageId("toggle-join"), blocks: [{
    id: noteBlockId("toggle"), type: "toggleListItem", props: {},
    content: {kind: "inline", items: [{type: "text", text: "Title", styles: []}]},
    children: [paragraph("left", "Left"), paragraph("right", "Right"), ...last ? [] : [paragraph("next", "Next")]],
  }]};
  let stored = {version: 1, saved: initial, baseline: initial};
  const session = await HbsSession.open({adapter: createNoteDocumentAdapter(directTextDiff), requestId: n => `toggle-${n}`,
    preservation: {load: () => undefined, save: () => {}}, port: {read: async () => stored, commit: async request => {
      stored = {version: stored.version + 1, saved: request.saved, baseline: request.baseline}; return stored;
    }},
  });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let editor: NoteEditorInstance | undefined;
  try {
    await act(async () => root.render(<NotesProvider gateway={gateway}>
      <HbsNoteEditor session={session} onEditorReady={value => {editor = value;}}/>
    </NotesProvider>));
    const incoming: NoteDocumentV1 = {...initial, blocks: initial.blocks.map(block => ({...block,
      children: [paragraph("left", "Left"), paragraph("right", "Incoming"), ...last ? [] : [paragraph("next", "Next")]],
    }))};
    stored = {...stored, version: 2, saved: incoming};
    await act(async () => session.syncLatest());
    await act(async () => {await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));});
    const native = editor!;
    await act(async () => native.setTextCursorPosition("right", "start"));
    await act(async () => native.domElement!.dispatchEvent(new KeyboardEvent("keydown", {key: "Backspace", bubbles: true, cancelable: true})));
    const joined = session.getSnapshot();
    if (last) {
      expect(native.getParentBlock("right")?.id, joined.message).toBeUndefined();
      expect(joined.content.blocks[0]?.children).toEqual([paragraph("left", "Left")]);
      expect(joined.content.blocks[1]).toEqual(paragraph("right", "Incoming"));
      // Cross-parent movement is an existing whole-document review boundary.
      // Keep accepted B intact, rather than implicitly accepting pending text.
      expect(joined.baselineProjection).toEqual(initial);
    } else {
      expect(joined.content.blocks[0]?.children).toEqual([paragraph("left", "LeftIncoming"), paragraph("next", "Next")]);
      expect(joined.baselineProjection.blocks[0]?.children).toEqual([paragraph("left", "LeftRight"), paragraph("next", "Next")]);
    }
    expect(joined.changes).toHaveLength(1);
    await act(async () => session.save());
    expect(stored.saved).toEqual(joined.content); expect(stored.baseline).toEqual(joined.baselineProjection);
    await act(async () => {native.undo();});
    expect(session.getSnapshot().content).toEqual(incoming); expect(session.getSnapshot().baselineProjection).toEqual(initial);
    await act(async () => {native.redo();});
    expect(session.getSnapshot().content).toEqual(joined.content);
    expect(session.getSnapshot().baselineProjection).toEqual(joined.baselineProjection);
  } finally {
    await act(async () => root.unmount()); session.dispose(); host.remove(); vi.unstubAllGlobals();
  }
});
