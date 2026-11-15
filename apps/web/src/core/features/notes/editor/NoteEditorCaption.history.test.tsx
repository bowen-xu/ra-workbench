// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {captionBlockId} from "../../../components/block-editor/captionBlockProjection";
import {captionNodeText} from "../../../components/block-editor/captionBlock";
import {textBlockLocation} from "../../../components/block-editor/blockTextEditing";
import {HbsSession, createNoteDocumentAdapter, type DocumentSnapshot} from "@research-assistant/core/hbs";
import {noteBlockId, notePageId, type NoteBlockV1, type NoteDocumentV1} from "@research-assistant/core/notes";
import {afterEach, expect, it, vi} from "vitest";
import {HbsNoteEditor} from "../../../components/hbs/HbsNoteEditor";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {noteRendererGateway} from "../../../../app/previews/hbs-adapters/noteGateway";
import {NotesProvider} from "../NotesClient";
import type {NoteEditorInstance} from "./noteEditorSchema";

vi.mock("../../../services/notifications", () => ({notify: {error: vi.fn(), info: vi.fn(), success: vi.fn()}}));
vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const dispose of cleanups.splice(0)) await dispose();
  vi.restoreAllMocks(); vi.unstubAllGlobals();
});
const png = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jI1sAAAAASUVORK5CYII="), value => value.charCodeAt(0));
type Media = "image" | "table";
function paragraph(text: string): NoteBlockV1 {
  return {id: noteBlockId("body"), type: "paragraph", props: {}, children: [],
    content: {kind: "inline", items: [{type: "text", text, styles: []}]}};
}
function initialDocument(type: Media): NoteDocumentV1 {
  const media: NoteBlockV1 = {id: noteBlockId("media"), type, children: [],
    props: type === "image" ? {textAlignment: "right", previewWidth: 273.5} : {caption: "原说明", textAlignment: "center"},
    content: type === "image" ? {kind: "asset", assetId: "a".repeat(64), displayName: "a".repeat(64) + ".png", mimeType: "image/png", caption: "原说明"}
      : {kind: "table", rows: [[{kind: "inline", items: [{type: "text", text: "Cell", styles: []}]}]]},
  };
  return {schemaVersion: 1, pageId: notePageId("caption-hbs"), blocks: [paragraph("Original body"), media]};
}
function caption(document: NoteDocumentV1): string {
  const block = document.blocks.find(block => block.id === "media");
  if (!block) throw new Error("missing caption owner");
  return block.content.kind === "asset" ? block.content.caption : String(block.props.caption ?? "");
}
function body(document: NoteDocumentV1) {return document.blocks.find(block => block.id === "body")?.content;}
function nativeText(editor: NoteEditorInstance): string {
  return captionNodeText(textBlockLocation(editor.prosemirrorView.state.doc, captionBlockId("media"))!.node.firstChild!);
}
async function frame() {await act(async () => {await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));});}
async function setup(type: Media, reopened?: DocumentSnapshot<NoteDocumentV1>) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:caption-hbs-image");
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
  const initial = initialDocument(type);
  let authority: DocumentSnapshot<NoteDocumentV1> = reopened ?? {version: 1, saved: initial, baseline: initial};
  const adapter = createNoteDocumentAdapter(directTextDiff);
  const commit = vi.fn(async (request: {readonly expectedVersion: number; readonly saved: NoteDocumentV1; readonly baseline: NoteDocumentV1}) => {
    expect(request.expectedVersion).toBe(authority.version);
    authority = {version: authority.version + 1, saved: request.saved, baseline: request.baseline}; return authority;
  });
  const session = await HbsSession.open({adapter, requestId: n => `caption-hbs-${n}`,
    preservation: {load: () => undefined, save: () => {}}, port: {read: async () => authority, commit}});
  const gateway = new Proxy(noteRendererGateway, {get(target, key) {
    return key === "readNoteAsset" ? async () => png.slice() : Reflect.get(target, key);
  }});
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let editor: NoteEditorInstance | undefined, mounted = true;
  const unmount = async () => {if (mounted) {await act(async () => root.unmount()); session.dispose(); host.remove(); mounted = false;}};
  cleanups.push(unmount);
  await act(async () => root.render(<NotesProvider gateway={gateway}>
    <HbsNoteEditor session={session} onEditorReady={value => {editor = value;}}/>
  </NotesProvider>));
  await frame();
  const label = type === "image" ? "图片说明" : "表格说明";
  const field = () => {
    const element = host.querySelector<HTMLElement>(`[role="textbox"][aria-label="${label}"]`);
    if (!element) throw new Error("native caption block not mounted"); return element;
  };
  await vi.waitFor(() => expect(host.querySelector(`[role="textbox"][aria-label="${label}"]`)).not.toBeNull());
  return {host, session, adapter, commit, field, unmount,
    get editor() {if (!editor) throw new Error("note editor not mounted"); return editor;},
    get saved() {return authority;},
    async input(value: string) {
      const dom = field();
      await act(async () => {
        editor!.setTextCursorPosition(captionBlockId("media"), "end"); editor!.focus();
        dom.textContent = value;
        dom.dispatchEvent(new InputEvent("input", {bubbles: true, inputType: "insertText", data: value}));
      });
      await frame(); expect(nativeText(editor!)).toBe(value);
    },
    async key(_key: string, shiftKey = false) {
      await act(async () => {if (shiftKey) editor!.redo(); else editor!.undo();});
      await frame();
    },
    async external(document: NoteDocumentV1) {
      authority = {...authority, version: authority.version + 1, saved: document};
      await act(async () => session.syncLatest()); await frame();
    },
    async save() {await act(async () => session.save()); await frame();},
  };
}
function expectClean(session: HbsSession<NoteDocumentV1>) {
  const state = session.getSnapshot(); expect(state.changes).toEqual([]); expect(state.conflicts).toEqual([]);
  expect(state.baselineProjection).toEqual(state.content);
}

for (const type of ["image", "table"] as const) {
  it(`routes each native ${type} caption input through ordinary HBS history and reopens multiline saved text`, async () => {
    const f = await setup(type), outer = f.editor.prosemirrorView, field = f.field(), table = f.host.querySelector("table");
    const update = vi.spyOn(f.editor, "updateBlock"), before = f.session.getSnapshot().content.blocks.find(block => block.id === "media")!;
    await f.input("手动说明");
    expect(update).not.toHaveBeenCalled();
    expect(caption(f.session.getSnapshot().content)).toBe("手动说明"); expectClean(f.session);
    await f.key("z");
    expect(caption(f.session.getSnapshot().content)).toBe("原说明"); expect(nativeText(f.editor)).toBe("原说明"); expectClean(f.session);
    await f.key("z", true);
    expect(caption(f.session.getSnapshot().content)).toBe("手动说明"); expect(nativeText(f.editor)).toBe("手动说明"); expectClean(f.session);
    await f.input("手动说明\n第二行 😀");
    expect(update.mock.calls.filter(([id, value]) => id === "media" && value.props && "caption" in value.props
      && value.props.caption === "手动说明\n第二行 😀")).toHaveLength(0);
    expectClean(f.session); expect(f.editor.prosemirrorView).toBe(outer); expect(f.field()).toBe(field);
    const after = f.session.getSnapshot().content.blocks.find(block => block.id === "media")!;
    expect(after).toEqual(type === "image" && before.content.kind === "asset"
      ? {...before, content: {...before.content, caption: "手动说明\n第二行 😀"}}
      : {...before, props: {...before.props, caption: "手动说明\n第二行 😀"}});
    if (type === "table") expect(f.host.querySelector("table")).toBe(table);
    await f.save(); expect(f.commit).toHaveBeenCalled();
    expect(caption(f.saved.saved)).toBe("手动说明\n第二行 😀"); expect(f.saved.baseline).toEqual(f.saved.saved);
    const saved = f.saved; await f.unmount(); const reopened = await setup(type, saved);
    expect(nativeText(reopened.editor)).toBe("手动说明\n第二行 😀"); expectClean(reopened.session);
  });

  it(`does not accept an unrelated agent change when manually editing, undoing or reopening a ${type} caption`, async () => {
    const f = await setup(type), initial = f.session.getSnapshot().content;
    const incoming = {...initial, blocks: initial.blocks.map(block => block.id === "body" ? paragraph("Agent proposed body") : block)};
    await f.external(incoming);
    const pending = f.session.getSnapshot().changes;
    expect(pending).toHaveLength(1); expect(body(f.session.getSnapshot().baselineProjection)).toEqual(body(initial));
    await f.input("本地说明\n仍待审阅");
    const expectPending = (value: string) => {
      const state = f.session.getSnapshot();
      expect(caption(state.content)).toBe(value); expect(caption(state.baselineProjection)).toBe(value);
      expect(body(state.content)).toEqual(body(incoming)); expect(body(state.baselineProjection)).toEqual(body(initial));
      expect(state.changes).toEqual(pending); expect(state.conflicts).toEqual([]);
    };
    expectPending("本地说明\n仍待审阅");
    await f.key("z"); expectPending("原说明");
    await f.key("z", true); expectPending("本地说明\n仍待审阅");
    await f.save(); expectPending("本地说明\n仍待审阅");
    expect(body(f.saved.saved)).toEqual(body(incoming)); expect(body(f.saved.baseline)).toEqual(body(initial));
    const saved = f.saved; await f.unmount(); const reopened = await setup(type, saved);
    expect(nativeText(reopened.editor)).toBe("本地说明\n仍待审阅");
    expect(reopened.session.getSnapshot().changes).toEqual(pending);
    expect(body(reopened.session.getSnapshot().baselineProjection)).toEqual(body(initial));
  });
}
