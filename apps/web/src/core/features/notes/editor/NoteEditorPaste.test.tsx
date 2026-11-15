// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {closeHistory} from "@tiptap/pm/history";
import {afterEach, expect, it, vi} from "vitest";
import {HbsSession, createNoteDocumentAdapter, type DocumentSnapshot} from "@research-assistant/core/hbs";
import {noteBlockId, notePageId, type NoteDocumentV1} from "@research-assistant/core/notes";
import {HbsNoteEditor} from "../../../components/hbs/HbsNoteEditor";
import {directTextDiff} from "../../../components/hbs/codeMirrorHbsPolicy";
import {NotesProvider} from "../NotesClient";
import {noteRendererGateway} from "../../../../app/previews/hbs-adapters/noteGateway";
import type {NoteEditorInstance} from "./noteEditorSchema";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const cleanup of cleanups.splice(0)) await cleanup(); vi.unstubAllGlobals();});

it("pastes colored browser HTML through the real HBS editor, saves and replays without accepting unrelated changes", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const paragraph = (id: string, text: string) => ({id: noteBlockId(id), type: "paragraph" as const, props: {}, children: [],
    content: {kind: "inline" as const, items: text ? [{type: "text" as const, text, styles: []}] : []}});
  const baseline: NoteDocumentV1 = {schemaVersion: 1, pageId: notePageId("paste-hbs"), title: "粘贴测试",
    blocks: [paragraph("body", ""), paragraph("pending", "原文")]};
  const initial = {...baseline, blocks: [baseline.blocks[0]!, paragraph("pending", "待审文字")]};
  let stored: DocumentSnapshot<NoteDocumentV1> = {version: 1, saved: initial, baseline};
  const options = {adapter: createNoteDocumentAdapter(directTextDiff), requestId: (n: number) => `paste-${n}`,
    preservation: {load: () => undefined, save: () => undefined}, port: {
      read: async () => stored,
      commit: async (request: {saved: NoteDocumentV1; baseline: NoteDocumentV1}) => {
        stored = {version: stored.version + 1, saved: request.saved, baseline: request.baseline}; return stored;
      },
    }};
  const session = await HbsSession.open(options);
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  let mounted: NoteEditorInstance | undefined;
  cleanups.push(async () => {await act(async () => root.unmount()); session.dispose(); host.remove();});
  await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={session} pageTitle={{value: "粘贴测试"}} onEditorReady={editor => {mounted = editor;}}/>
  </NotesProvider>));
  await act(async () => {await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));});
  if (!mounted) throw new Error("paste_fixture_not_mounted");
  const editor = mounted;
  expect(session.getSnapshot().changes).toHaveLength(1);
  await act(async () => {
    editor.setTextCursorPosition("body", "start");
    editor.prosemirrorView.dispatch(closeHistory(editor.prosemirrorState.tr));
    const formats: Record<string, string> = {"text/html": '<p><span style="color: rgb(32, 33, 36); background-color: #ffffff">网页<strong>粗体</strong>和<a href="https://example.org">链接</a></span></p>',
      "text/plain": "网页粗体和链接"};
    const event = new Event("paste", {bubbles: true, cancelable: true});
    Object.defineProperty(event, "clipboardData", {value: {types: Object.keys(formats), files: [], items: [], getData: (type: string) => formats[type] ?? ""}});
    editor.prosemirrorView.dom.dispatchEvent(event);
  });
  const pasted = session.getSnapshot().content;
  expect(pasted.blocks[0]?.content).toMatchObject({kind: "inline", items: [
    {type: "text", text: "网页", styles: []}, {type: "text", text: "粗体", styles: ["bold"]},
    {type: "text", text: "和", styles: []}, {type: "link", href: "https://example.org", content: [{text: "链接", styles: []}]},
  ]});
  expect(session.getSnapshot().changes).toHaveLength(1);
  expect(session.getSnapshot().baselineProjection.blocks[1]).toEqual(baseline.blocks[1]);
  await act(async () => session.save());
  expect(stored.saved).toEqual(pasted);
  await act(async () => {expect(editor.undo()).toBe(true);});
  expect(session.getSnapshot().content).toEqual(initial);
  expect(session.getSnapshot().changes).toHaveLength(1);
  await act(async () => {expect(editor.redo()).toBe(true); await session.save();});
  expect(session.getSnapshot().content).toEqual(pasted);
  const reopened = await HbsSession.open(options);
  expect(reopened.getSnapshot().content).toEqual(pasted);
  expect(reopened.getSnapshot().changes).toHaveLength(1);
  reopened.dispose();
});
