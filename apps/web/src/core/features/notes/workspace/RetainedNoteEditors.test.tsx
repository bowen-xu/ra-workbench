/** @vitest-environment happy-dom */
import {act, useEffect} from "react";
import {createRoot} from "react-dom/client";
import {afterEach, expect, it, vi} from "vitest";
import {notePageId} from "@research-assistant/core/notes";
import type {NoteEditorInstallPort} from "../editor/noteEditorInstallPort";
import type {RetainedNoteSession} from "./retainedNoteSession";
import {RetainedNoteEditors} from "./RetainedNoteEditors";

const f = vi.hoisted(() => ({mounted: vi.fn(), disposed: vi.fn()}));
vi.mock("../editor/NoteEditor", () => ({NoteEditor: (props: {document: {pageId: string}; onInstallPort: (port: NoteEditorInstallPort | undefined) => void}) => {
  useEffect(() => {
    f.mounted(props.document.pageId);
    props.onInstallPort({pageId: props.document.pageId} as unknown as NoteEditorInstallPort);
    return () => {f.disposed(props.document.pageId); props.onInstallPort(undefined);};
  }, []);
  return <textarea aria-label={props.document.pageId} defaultValue={props.document.pageId}/>;
}}));
vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
let root: ReturnType<typeof createRoot> | undefined, host: HTMLDivElement;
afterEach(async () => {if (root) await act(async () => root!.unmount()); host?.remove(); vi.clearAllMocks();});
function entry(id: string, editorSessionId = 1): RetainedNoteSession {
  const document = {pageId: notePageId(id), schemaVersion: 1, blocks: []};
  const state = {content:document};
  return {snapshot: {page: {id: notePageId(id), status: "active"}, document: {pageId: notePageId(id), schemaVersion: 1, blocks: []}},
    editorSessionId, saveStatus: "saved", saveMessage: undefined, outgoing: [], incoming: [],
    hbs: {getSnapshot: () => state,subscribe:()=>()=>{}},
    controller: {}, unregisterText: undefined, recoveryTimer: undefined, invalidated: false} as unknown as RetainedNoteSession;
}
it("retains actual editor hosts and their local input across switching and Home, releases only closed sessions", async () => {
  host = document.createElement("div"); document.body.append(host); root = createRoot(host);
  const a = entry("a"), b = entry("b", 2), install = vi.fn();
  async function show(activeId: string | undefined, sessions = [a, b]) {
    await act(async () => root!.render(<RetainedNoteEditors sessions={sessions} activeId={activeId} conflictLocked={false}
      onChange={vi.fn()} onInstallPort={install}/>));
  }
  await show("a", [a]);
  const editor = host.querySelector<HTMLTextAreaElement>('[aria-label="a"]')!; editor.value = "uncommitted local editor state";
  await show("b");
  expect(editor.parentElement!.hidden).toBe(true);
  expect(editor.parentElement!.hasAttribute("inert")).toBe(true);
  await show(undefined);
  await show("a");
  expect(host.querySelector('[aria-label="a"]')).toBe(editor);
  expect(editor.value).toBe("uncommitted local editor state");
  expect(editor.parentElement!.hidden).toBe(false);
  expect(install).toHaveBeenLastCalledWith({pageId: "a"});
  expect(f.mounted).toHaveBeenCalledTimes(2); expect(f.disposed).not.toHaveBeenCalled();
  await show("b", [b]);
  expect(f.disposed).toHaveBeenCalledExactlyOnceWith("a");
  expect(host.querySelector('[aria-label="a"]')).toBeNull();
});
