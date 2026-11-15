// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {Plugin, TextSelection} from "@tiptap/pm/state";
import {closeHistory, undo as nativeUndo} from "@tiptap/pm/history";
import {afterEach, expect, it, vi} from "vitest";
import {HbsSession, createNoteDocumentAdapter, type DocumentSnapshot} from "@research-assistant/core/hbs";
import {noteBlockId, notePageId, type NoteDocumentV1} from "@research-assistant/core/notes";
import {NotesProvider} from "../../features/notes/NotesClient";
import type {NoteEditorInstance} from "../../features/notes/editor/noteEditorSchema";
import {noteRendererGateway} from "../../../app/previews/hbs-adapters/noteGateway";
import {directTextDiff} from "./codeMirrorHbsPolicy";
import {HbsNoteEditor} from "./HbsNoteEditor";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const cleanup of cleanups.splice(0)) await cleanup(); vi.unstubAllGlobals();});
const text = "## 1. 给定规则，使用穷举算法完成目标";
async function setup(candidate = false) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const initial: NoteDocumentV1 = {schemaVersion: 1, pageId: notePageId("input-rule-history"), title: "标题",
    blocks: [{id: noteBlockId("body"), type: "paragraph", props: {}, children: [], content: {kind: "inline", items: []}},
      ...(candidate ? [{id: noteBlockId("pending"), type: "paragraph" as const, props: {}, children: [], content: {kind: "inline" as const, items: [{type: "text" as const, text: "原文", styles: []}]}}] : [])]};
  const current = candidate ? {...initial, blocks: [initial.blocks[0]!, {...initial.blocks[1]!, content: {kind: "inline" as const, items: [{type: "text" as const, text: "外部候选", styles: []}]}}]} : initial;
  let authority: DocumentSnapshot<NoteDocumentV1> = {version: 1, saved: current, baseline: initial};
  const preservation = vi.fn(), commit = vi.fn(async (request: {saved: NoteDocumentV1; baseline: NoteDocumentV1}) => authority = {version: authority.version + 1, saved: request.saved, baseline: request.baseline});
  const session = await HbsSession.open({adapter: createNoteDocumentAdapter(directTextDiff), requestId: n => `input-rule-${n}`,
    preservation: {load: () => undefined, save: preservation}, port: {read: async () => authority, commit}});
  const host = document.createElement("div"), controls = document.createElement("div"); document.body.append(host, controls);
  const root = createRoot(host); let editor: NoteEditorInstance | undefined;
  cleanups.push(async () => {await act(async () => root.unmount()); session.dispose(); host.remove(); controls.remove();});
  await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={session} pageTitle={{value: "标题"}} diffControlsTarget={controls} onEditorReady={value => {editor = value;}}/>
  </NotesProvider>));
  await act(async () => {await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));});
  if (!editor) throw new Error("input_rule_fixture_not_mounted");
  preservation.mockClear(); commit.mockClear();
  return {editor, session, controls, preservation, commit};
}
async function paste(f: Awaited<ReturnType<typeof setup>>, html: string, plain = text) {
  await act(async () => {
    f.editor.setTextCursorPosition("body", "start");
    const formats: Record<string, string> = {"text/plain": plain, ...(html ? {"text/html": html} : {})};
    const event = new Event("paste", {bubbles: true, cancelable: true});
    Object.defineProperty(event, "clipboardData", {value: {types: Object.keys(formats), files: [], items: [], getData: (type: string) => formats[type] ?? ""}});
    f.editor.prosemirrorView.dom.dispatchEvent(event);
  });
}
async function formatHeading(f: Awaited<ReturnType<typeof setup>>) {
  await act(async () => {
    const view = f.editor.prosemirrorView;
    let position = -1;
    view.state.doc.descendants((node, pos) => {if (node.isTextblock && node.textContent === text) position = pos + 3;});
    expect(position).toBeGreaterThan(0);
    view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, position)));
    const handled = view.someProp("handleTextInput", handler => handler(view, position, position, " ", () => view.state.tr.insertText(" ", position)));
    if (!handled) view.dispatch(view.state.tr.insertText(" ", position));
  });
}
it.each([
  {label: "bold paragraph grouped", html: `<p><strong>${text}</strong></p>`, save: "none", grouped: true},
  {label: "bold paragraph", html: `<p><strong>${text}</strong></p>`, save: "none"},
  {label: "bold body", html: `<p>## <strong>1. 给定规则，使用穷举算法完成目标</strong></p>`, save: "none"},
  {label: "plain HTML paragraph", html: `<p>${text}</p>`, save: "none"},
  {label: "raw Markdown bold", html: "", plain: `**${text}**`, save: "none"},
  {label: "bold paragraph", html: `<p><strong>${text}</strong></p>`, save: "paste"},
  {label: "bold paragraph", html: `<p><strong>${text}</strong></p>`, save: "heading"},
  {label: "bold paragraph", html: `<p><strong>${text}</strong></p>`, save: "each"},
])("keeps pasted $label and heading input-rule Undo ordinary (save: $save)", async ({html, plain, save, grouped}) => {
  const f = await setup(), view = f.editor.prosemirrorView;
  await paste(f, html, plain);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.session.getSnapshot().content.blocks[0]?.type).toBe("paragraph");
  if (save === "paste" || save === "each") await act(async () => f.session.save());
  if (!grouped) await act(async () => view.dispatch(closeHistory(view.state.tr)));
  await formatHeading(f);
  expect(f.session.getSnapshot().content.blocks[0]?.type).toBe("heading");
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  const formatted = f.session.getSnapshot().content;
  if (save === "heading" || save === "each") await act(async () => f.session.save());
  for (let undo = 0; undo < 3; undo++) {
    await act(async () => {f.editor.undo();});
    if (save === "each") await act(async () => f.session.save());
    const current = f.session.getSnapshot();
    expect(current.conflicts, `conflicts after Undo ${undo + 1}`).toHaveLength(0);
    expect(current.changes, `changes after Undo ${undo + 1}`).toHaveLength(0);
    expect(current.baselineProjection, `baseline after Undo ${undo + 1}`).toEqual(current.content);
    expect(f.controls.querySelector(".note-editor-diff-controls")).toBeNull();
  }
  for (let redo = 0; redo < 3; redo++) {
    await act(async () => {f.editor.redo();});
    if (save === "each") await act(async () => f.session.save());
    expect(f.session.getSnapshot().changes).toHaveLength(0);
    expect(f.session.getSnapshot().conflicts).toHaveLength(0);
    expect(f.session.getSnapshot().baselineProjection).toEqual(f.session.getSnapshot().content);
  }
  expect(f.session.getSnapshot().content).toEqual(formatted);
});

it.each([false, true])("preserves unrelated candidates through input-rule Undo3 and Redo3 (save each: %s)", async save => {
  const f = await setup(true), view = f.editor.prosemirrorView, pending = f.session.getSnapshot().changes;
  await paste(f, `<p><strong>${text}</strong></p>`);
  if (save) await act(async () => f.session.save());
  await act(async () => view.dispatch(closeHistory(view.state.tr)));
  await formatHeading(f);
  if (save) await act(async () => f.session.save());
  const formatted = f.session.getSnapshot().content;
  for (const direction of ["undo", "redo"] as const) for (let index = 0; index < 3; index++) {
    await act(async () => {f.editor[direction]();});
    if (save) await act(async () => f.session.save());
    const state = f.session.getSnapshot();
    expect(state.changes).toEqual(pending);
    expect(state.conflicts).toHaveLength(0);
    expect(state.baselineProjection.blocks[0]).toEqual(state.content.blocks[0]);
    expect(state.baselineProjection.blocks[1]?.content).toMatchObject({items: [{text: "原文"}]});
    expect(state.content.blocks[1]?.content).toMatchObject({items: [{text: "外部候选"}]});
  }
  expect(f.session.getSnapshot().content).toEqual(formatted);
  expect(f.editor.prosemirrorView).toBe(view);
});

it("does not publish or preserve a speculative input/history state until the view commits it", async () => {
  const f = await setup(), view = f.editor.prosemirrorView, updates = vi.fn();
  await act(async () => f.editor.setTextCursorPosition("body", "start"));
  const stop = f.session.subscribe(updates);
  try {
    const before = f.session.getSnapshot(), native = view.state, transaction = native.tr.insertText("试算");
    const trial = native.applyTransaction(transaction);
    expect(trial.state.doc).not.toEqual(native.doc);
    expect(view.state).toBe(native);
    expect(f.session.getSnapshot()).toBe(before);
    expect(updates).not.toHaveBeenCalled(); expect(f.preservation).not.toHaveBeenCalled(); expect(f.commit).not.toHaveBeenCalled();
    await act(async () => view.dispatch(transaction));
    expect(f.session.getSnapshot().content.blocks[0]?.content).toMatchObject({items: [{text: "试算"}]});
    expect(f.session.getSnapshot().baselineProjection).toEqual(f.session.getSnapshot().content);
    updates.mockClear(); f.preservation.mockClear();
    const applied = f.session.getSnapshot(); let undo: Parameters<typeof view.dispatch>[0] | undefined;
    nativeUndo(view.state, transaction => {undo = transaction;});
    expect(undo).toBeDefined();
    view.state.applyTransaction(undo!);
    expect(f.session.getSnapshot()).toBe(applied);
    expect(updates).not.toHaveBeenCalled(); expect(f.preservation).not.toHaveBeenCalled(); expect(f.commit).not.toHaveBeenCalled();
    await act(async () => view.dispatch(undo!));
    expect(f.session.getSnapshot().changes).toHaveLength(0);
    expect(f.session.getSnapshot().content.blocks[0]?.content).toMatchObject({items: []});
    updates.mockClear();
    await act(async () => view.dispatch(view.state.tr.setSelection(TextSelection.create(view.state.doc, view.state.selection.from))));
    expect(updates).not.toHaveBeenCalled();
  } finally {stop();}
});

it("does not republish retained history when another editor plugin is reconfigured", async () => {
  const f = await setup(), view = f.editor.prosemirrorView;
  await act(async () => {f.editor.setTextCursorPosition("body", "start"); view.dispatch(view.state.tr.insertText("输入"));});
  const updates = vi.fn(), stop = f.session.subscribe(updates);
  try {
    for (const replay of [false, true]) {
      if (replay) await act(async () => {f.editor.undo();});
      const before = f.session.getSnapshot(); updates.mockClear(); f.preservation.mockClear(); f.commit.mockClear();
      await act(async () => f.editor.registerExtension({key: "history-view-reconfigure-probe", prosemirrorPlugins: [new Plugin({})]}));
      await act(async () => f.editor.unregisterExtension("history-view-reconfigure-probe"));
      expect(f.editor.prosemirrorView).toBe(view);
      expect(f.session.getSnapshot()).toBe(before);
      expect(updates).not.toHaveBeenCalled(); expect(f.preservation).not.toHaveBeenCalled(); expect(f.commit).not.toHaveBeenCalled();
    }
  } finally {stop();}
});

it("publishes one complete native batch across repeated appended normalizer passes", async () => {
  const f = await setup(), view = f.editor.prosemirrorView;
  await act(async () => f.editor.setTextCursorPosition("body", "start"));
  const normalize = (expected: string, suffix: string) => new Plugin({appendTransaction(_transactions, _before, after) {
    let position: number | undefined;
    after.doc.descendants((node, pos) => {if (node.isTextblock && node.textContent === expected) position = pos + 1 + expected.length;});
    return position === undefined ? null : after.tr.insertText(suffix, position);
  }});
  const plugins = view.state.plugins;
  await act(async () => view.updateState(view.state.reconfigure({plugins: [...plugins,
    new Plugin({appendTransaction(_transactions, _before, after) {
      let position: number | undefined, suffix = "";
      after.doc.descendants((node, pos) => {
        if (node.isTextblock && ["abc", "abc12"].includes(node.textContent)) {
          position = pos + 1 + node.textContent.length; suffix = node.textContent === "abc" ? "1" : "3";
        }
      });
      return position === undefined ? null : after.tr.insertText(suffix, position);
    }}), normalize("abc1", "2")]})));
  const before = f.session.getSnapshot(), transaction = view.state.tr.insertText("abc");
  const trial = view.state.applyTransaction(transaction);
  const steps = trial.transactions.flatMap(item => item.steps).filter(step => (step.toJSON() as {stepType?: string}).stepType === "hbsSessionV1");
  expect(steps).toHaveLength(1);
  expect(f.session.getSnapshot()).toBe(before);
  await act(async () => view.dispatch(transaction));
  expect(f.session.getSnapshot().content.blocks[0]?.content).toMatchObject({items: [{text: "abc123"}]});
  expect(f.session.getSnapshot().baselineProjection).toEqual(f.session.getSnapshot().content);
  await act(async () => view.updateState(view.state.reconfigure({plugins})));
  const restore = vi.spyOn(f.session, "restore");
  await act(async () => {expect(f.editor.undo()).toBe(true);});
  expect(restore).toHaveBeenCalledTimes(1);
  expect(f.session.getSnapshot().content.blocks[0]?.content).toMatchObject({items: []});
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});
