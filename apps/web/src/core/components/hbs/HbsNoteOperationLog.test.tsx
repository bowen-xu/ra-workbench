// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {undoDepth} from "@tiptap/pm/history";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {createNoteDocumentAdapter, HbsSession, type DocumentSnapshot} from "@research-assistant/core/hbs";
import type {NoteDocumentV1} from "@research-assistant/core/notes";
import {NotesProvider} from "../../features/notes/NotesClient";
import type {NoteEditorInstance} from "../../features/notes/editor/noteEditorSchema";
import {DeveloperModeProvider, DeveloperModeToggle} from "../../features/settings/DeveloperMode";
import {noteFixture} from "../../../app/previews/hbs-adapters/fixtures";
import {noteRendererGateway} from "../../../app/previews/hbs-adapters/noteGateway";
import {directTextDiff} from "./codeMirrorHbsPolicy";
import {HbsNoteEditor} from "./HbsNoteEditor";
import {HbsOperationLog} from "./operationLog";
import {useNoteOperationLogActions, type NoteOperationLogActions} from "./noteOperationLogActions";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const feedback = vi.hoisted(() => ({success: vi.fn(), error: vi.fn(), info: vi.fn()}));
vi.mock("../../services/notifications", () => ({notify: feedback}));
const cleanups: (() => Promise<void>)[] = [];
beforeEach(() => {vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); localStorage.removeItem("research-assistant.developer-mode");});
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) await cleanup();
  localStorage.removeItem("research-assistant.developer-mode"); vi.restoreAllMocks(); vi.unstubAllGlobals();
});
async function fixture(beforeOpen?: (session: HbsSession<NoteDocumentV1>) => void) {
  const adapter = createNoteDocumentAdapter(directTextDiff), initial = {...noteFixture, blocks: [noteFixture.blocks[1]! ]};
  let observed: DocumentSnapshot<NoteDocumentV1> = {version: 1, saved: initial, baseline: initial};
  const commit = vi.fn(async request => {observed = {version: observed.version + 1, saved: request.saved, baseline: request.baseline}; return observed;});
  const session = await HbsSession.open({adapter, port: {read: async () => observed, commit},
    preservation: {load: async () => undefined, save: async () => {}}, requestId: sequence => `capture:${sequence}`});
  beforeOpen?.(session);
  let editor: NoteEditorInstance | undefined;
  let actions: NoteOperationLogActions | undefined;
  function ActionsProbe() {actions = useNoteOperationLogActions(initial.pageId); return null;}
  const host = document.createElement("div"), toolbar = document.createElement("div");
  document.body.append(toolbar, host); const root = createRoot(host);
  let mounted = true;
  const unmount = async () => {if (mounted) {await act(async () => root.unmount()); mounted = false;}};
  cleanups.push(async () => {await unmount(); session.dispose(); toolbar.remove(); host.remove();});
  await act(async () => root.render(<DeveloperModeProvider><DeveloperModeToggle/><NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={session} diffControlsTarget={toolbar} onEditorReady={value => {editor = value;}}/>
    <ActionsProbe/>
  </NotesProvider></DeveloperModeProvider>));
  await act(async () => {await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));});
  if (!editor) throw new Error("editor not ready");
  const native = editor.domElement!;
  const toggle = async () => {await act(async () => host.querySelector<HTMLInputElement>('.developer-mode-toggle input')!.click());};
  return {host, toolbar, editor, native, session, commit, toggle, unmount, get actions() {return actions;}};
}

it("captures only while developer mode is enabled and releases records/subscriptions on disable and unmount", async () => {
  const observe = vi.spyOn(HbsOperationLog.prototype, "observe"), record = vi.spyOn(HbsOperationLog.prototype, "record");
  const f = await fixture();
  expect(f.toolbar.querySelector('[aria-label="笔记操作记录"]')).toBeNull();
  expect(f.actions).toBeUndefined();
  await act(async () => f.editor.updateBlock("hbs-score", {content: "before enable"}));
  expect(observe).not.toHaveBeenCalled(); expect(record).not.toHaveBeenCalled();
  await f.toggle();
  expect(f.toolbar.querySelector('[aria-label="笔记操作记录"]')).toBeNull();
  expect(f.actions).toBeDefined();
  expect(observe).toHaveBeenCalledTimes(1);
  const log = observe.mock.contexts[0] as HbsOperationLog;
  expect(log.getSnapshot().filter(entry => entry.label === "会话开始")).toHaveLength(1);
  await act(async () => f.editor.updateBlock("hbs-score", {content: "captured input"}));
  expect(log.exportJson()).toContain("captured input");
  await f.toggle();
  expect(f.actions).toBeUndefined();
  expect(log.getSnapshot()).toEqual([]); const calls = record.mock.calls.length;
  await act(async () => {
    f.native.dispatchEvent(new InputEvent("beforeinput", {bubbles: true, inputType: "insertText", data: "unrecorded"}));
    f.editor.updateBlock("hbs-score", {content: "disabled input"});
  });
  expect(record.mock.calls).toHaveLength(calls);
  await f.toggle();
  expect(observe).toHaveBeenCalledTimes(2);
  expect(log.getSnapshot().filter(entry => entry.label === "会话开始")).toHaveLength(1);
  expect(log.exportJson()).not.toContain("captured input");
  await f.unmount(); expect(log.getSnapshot()).toEqual([]);
  const after = record.mock.calls.length; f.session.edit({...f.session.getSnapshot().content, title: "after-unmount"});
  expect(record.mock.calls).toHaveLength(after); expect(f.commit).not.toHaveBeenCalled();
});

it("starts from opening the note and never backfills earlier model events", async () => {
  localStorage.setItem("research-assistant.developer-mode", "true");
  const observe = vi.spyOn(HbsOperationLog.prototype, "observe");
  const f = await fixture(session => session.record("before-note-open"));
  const log = observe.mock.contexts[0] as HbsOperationLog;
  expect(f.actions).toBeDefined();
  expect(log.getSnapshot().map(entry => entry.label)).toEqual(["会话开始", "原生编辑器连接"]);
  expect(log.exportJson()).not.toContain("before-note-open");
  await act(async () => f.session.record("after-note-open"));
  expect(log.exportJson()).toContain("after-note-open");
  expect(f.commit).not.toHaveBeenCalled();
});

it("discards the closed note's log and starts fresh when the same note opens again", async () => {
  localStorage.setItem("research-assistant.developer-mode", "true");
  const observe = vi.spyOn(HbsOperationLog.prototype, "observe");
  const first = await fixture();
  const firstLog = observe.mock.contexts[0] as HbsOperationLog;
  await act(async () => first.native.dispatchEvent(new InputEvent("beforeinput", {
    bubbles: true, inputType: "insertText", data: "previous-opening-only",
  })));
  expect(firstLog.exportJson()).toContain("previous-opening-only");
  await first.unmount();
  expect(firstLog.getSnapshot()).toEqual([]);
  const reopened = await fixture();
  const nextLog = observe.mock.contexts[1] as HbsOperationLog;
  expect(nextLog).not.toBe(firstLog);
  expect(reopened.actions).toBeDefined();
  expect(nextLog.getSnapshot().map(entry => entry.label)).toEqual(["会话开始", "原生编辑器连接"]);
  expect(nextLog.exportJson()).not.toContain("previous-opening-only");
  expect(nextLog.getSnapshot()[0]?.sequence).toBe(1);
});

it("exports paste, native input and undo ordering through actions without rendering details or mutating document/history", async () => {
  const observe = vi.spyOn(HbsOperationLog.prototype, "observe"); const f = await fixture(); await f.toggle();
  const log = observe.mock.contexts[0] as HbsOperationLog;
  const text = "## 1. 给定规则，使用穷举算法完成目标";
  const getData = vi.fn((type: string) => type === "text/plain" ? text : type === "text/html" ? `<b>${text}</b>` : type === "application/private" ? "not permitted" : "");
  const paste = new Event("paste", {bubbles: true, cancelable: true}); Object.defineProperty(paste, "clipboardData", {value: {getData}});
  // Inspect capture without also asking the clipboard engine to process this synthetic payload.
  f.native.addEventListener("paste", event => {event.preventDefault(); event.stopImmediatePropagation();}, {capture: true, once: true});
  await act(async () => {
    f.native.dispatchEvent(paste);
    f.native.dispatchEvent(new InputEvent("beforeinput", {bubbles: true, inputType: "insertText", data: " "}));
    f.editor.updateBlock("hbs-score", {content: text});
    f.native.dispatchEvent(new KeyboardEvent("keydown", {key: "z", ctrlKey: true, bubbles: true, cancelable: true}));
  });
  // Existing native clipboard adapters may read their own private format;
  // diagnostic payloads only retain the explicitly supported plain/HTML data.
  expect(getData.mock.calls.map(call => call[0])).not.toContain("application/private");
  expect(log.getSnapshot().find(entry => entry.label === "粘贴事件")?.detail).toMatchObject({plainText: text, html: `<b>${text}</b>`});
  const exported = JSON.parse(log.exportJson());
  expect(exported).toMatchObject({scope: "note-editor", capture: {complete: true}});
  const labels = log.getSnapshot().map(entry => entry.label);
  expect(labels.indexOf("粘贴事件")).toBeLessThan(labels.indexOf("输入前事件"));
  expect(labels.indexOf("输入前事件")).toBeLessThan(labels.indexOf("原生事务批次"));
  expect(labels).toContain("触发撤销");
  const captured = log.getSnapshot().length;
  await act(async () => {
    document.body.dispatchEvent(new KeyboardEvent("keydown", {key: "z", ctrlKey: true, bubbles: true}));
    document.body.dispatchEvent(new InputEvent("beforeinput", {data: "outside-note", bubbles: true}));
  });
  expect(log.getSnapshot()).toHaveLength(captured);
  expect(log.exportJson()).not.toContain("outside-note");
  const batches = log.getSnapshot().filter(entry => entry.label === "原生事务批次").map(entry => entry.detail as {transactions: {history: boolean; steps: {stepType: string; entryId?: number}[]}[]});
  expect(batches.some(batch => batch.transactions.some(transaction => transaction.history))).toBe(true);
  expect(batches.some(batch => batch.transactions.length > 1)).toBe(true);
  expect(batches.flatMap(batch => batch.transactions).flatMap(transaction => transaction.steps).some(step => step.stepType === "hbsSessionV1" && step.entryId)).toBe(true);
  expect(log.exportJson()).not.toContain("not permitted");
  const before = f.session.getSnapshot(), depth = undoDepth(f.editor.prosemirrorState), view = f.editor.prosemirrorView;
  const writeText = vi.fn().mockResolvedValue(undefined);
  const prior = Object.getOwnPropertyDescriptor(navigator, "clipboard");
  Object.defineProperty(navigator, "clipboard", {configurable: true, value: {writeText}});
  try {
    await act(async () => f.actions!.copy());
    expect(writeText).toHaveBeenCalledWith(log.exportJson());
  } finally {
    if (prior) Object.defineProperty(navigator, "clipboard", prior);
    else Reflect.deleteProperty(navigator, "clipboard");
  }
  expect(document.querySelector('[role="dialog"][aria-label="笔记操作记录"]')).toBeNull();
  expect(document.querySelector('.hbs-operation-log')).toBeNull();
  expect(f.session.getSnapshot()).toBe(before); expect(undoDepth(f.editor.prosemirrorState)).toBe(depth); expect(f.editor.prosemirrorView).toBe(view);
  expect(f.commit).not.toHaveBeenCalled();
  await act(async () => f.actions!.clear());
  expect(log.getSnapshot().filter(entry => entry.label === "会话开始")).toHaveLength(1);
  expect(log.exportJson()).not.toContain("粘贴事件");
  expect(feedback.success).toHaveBeenCalledWith("已清空操作记录");
  expect(f.session.getSnapshot()).toBe(before); expect(undoDepth(f.editor.prosemirrorState)).toBe(depth);
});
