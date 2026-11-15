// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { closeHistory } from "@tiptap/pm/history";
import { afterEach, expect, it, vi } from "vitest";
import { HbsSession, createNoteDocumentAdapter, type DocumentCheckpoint, type DocumentCommit, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { noteBlockId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { moveBlocks } from "../block-editor";
import { NotesProvider } from "../../features/notes/NotesClient";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import { noteBlockMovePolicy } from "../../features/notes/editor/noteBlockMovePolicy";
import { noteFixture } from "../../../app/previews/hbs-adapters/fixtures";
import { noteRendererGateway } from "../../../app/previews/hbs-adapters/noteGateway";
import { directTextDiff } from "./codeMirrorHbsPolicy";
import { HbsNoteEditor } from "./HbsNoteEditor";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", { configurable: true, value: "CSS1Compat" });
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => { for (const cleanup of cleanups.splice(0)) await cleanup(); vi.unstubAllGlobals(); });
const ids = (value: NoteDocumentV1) => value.blocks.map(block => block.id);
function assertOrdinary(session: HbsSession<NoteDocumentV1>): void {
  expect(session.getSnapshot().conflicts).toHaveLength(0);
  expect(session.getSnapshot().changes).toHaveLength(0);
}
async function setup(initial = noteFixture, baseline = initial) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const adapter = createNoteDocumentAdapter(directTextDiff);
  let saved: DocumentSnapshot<NoteDocumentV1> = { version: 1, saved: initial, baseline };
  let editor: NoteEditorInstance | undefined, checkpoint: DocumentCheckpoint<NoteDocumentV1> | undefined;
  let holdNext = false, release: (() => void) | undefined;
  const options = {
    adapter, requestId: (sequence: number) => `native-structure-${sequence}`,
    preservation: { load: () => checkpoint, save: (value: DocumentCheckpoint<NoteDocumentV1>) => { checkpoint = structuredClone(value); } },
    port: { read: async () => saved, commit: async (request: DocumentCommit<NoteDocumentV1>) => {
      expect(request.expectedVersion).toBe(saved.version);
      saved = { version: saved.version + 1, saved: request.saved, baseline: request.baseline };
      const receipt = saved;
      if (holdNext) { holdNext = false; await new Promise<void>(resolve => { release = resolve; }); }
      return receipt;
    } },
  };
  const session = await HbsSession.open(options), host = document.createElement("div");
  document.body.append(host); const root = createRoot(host);
  cleanups.push(async () => { await act(async () => root.unmount()); session.dispose(); host.remove(); });
  await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={session} onEditorReady={value => { editor = value; }}/>
  </NotesProvider>));
  await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
  return { session, options, hold: () => { holdNext = true; }, release: () => release?.(),
    get editor() { if (!editor) throw new Error("editor not mounted"); return editor; } };
}

it.each(["before", "after"] as const)("keeps native insertion %s a formula ordinary through save, Undo and Redo", async side => {
  const f = await setup();
  await act(async () => { f.editor.insertBlocks([{ id: "ordinary-new", type: "paragraph", content: "1" }], "hbs-formula", side); });
  await act(async () => f.session.save()); assertOrdinary(f.session);
  await act(async () => { expect(f.editor.undo()).toBe(true); }); assertOrdinary(f.session);
  await act(async () => { await f.session.save(); expect(f.editor.redo()).toBe(true); await f.session.save(); });
  assertOrdinary(f.session);
});

it("keeps grouped native insertion and typing ordinary after save and Undo", async () => {
  const f = await setup();
  await act(async () => { f.editor.insertBlocks([{ id: "ordinary-new", type: "paragraph", content: "" }], "hbs-formula", "after"); });
  await act(async () => { f.editor.updateBlock("ordinary-new", { content: "1" }); });
  await act(async () => f.session.save()); assertOrdinary(f.session);
  await act(async () => { expect(f.editor.undo()).toBe(true); }); assertOrdinary(f.session);
});

it.each([
  {saved: false, candidate: false}, {saved: true, candidate: false},
  {saved: false, candidate: true}, {saved: true, candidate: true},
])("replays every native group delta (saved: $saved, unrelated candidate: $candidate)", async ({saved, candidate}) => {
  const initial = candidate ? createNoteDocumentAdapter(directTextDiff).replace(noteFixture,
    ["blocks", {key: "id", value: "hbs-score"}, "content", "items", 0, "text"], "模型准确率为 85%。") : noteFixture;
  const f = await setup(initial, noteFixture);
  await act(async () => {
    f.editor.insertBlocks([{id: "ordinary-first", type: "paragraph", content: "123"}], "hbs-formula", "after");
    f.editor.setTextCursorPosition("ordinary-first", "end");
    f.editor.transact(transaction => { closeHistory(transaction); });
  });
  const before = f.session.getSnapshot().content;
  const beforeBaseline = f.session.getSnapshot().baselineProjection;
  const beforeChanges = f.session.getSnapshot().changes;
  await act(async () => {
    const view = f.editor.prosemirrorView;
    const enter = () => view.dom.dispatchEvent(new KeyboardEvent("keydown", {key: "Enter", code: "Enter", keyCode: 13, bubbles: true, cancelable: true}));
    enter();
    view.dispatch(view.state.tr.insertText("1"));
    enter();
  });
  const typed = f.session.getSnapshot().content;
  const typedBaseline = f.session.getSnapshot().baselineProjection;
  if (saved) await act(async () => f.session.save());
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(f.session.getSnapshot().content).toEqual(before);
  expect(f.session.getSnapshot().baselineProjection).toEqual(beforeBaseline);
  expect(f.session.getSnapshot().changes).toEqual(beforeChanges);
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  await act(async () => { expect(f.editor.redo()).toBe(true); });
  expect(f.session.getSnapshot().content).toEqual(typed);
  expect(f.session.getSnapshot().baselineProjection).toEqual(typedBaseline);
  expect(f.session.getSnapshot().changes).toEqual(beforeChanges);
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
});

it.each(["before-ack", "after-ack"] as const)("recovers an unsaved native Undo %s without creating candidates", async phase => {
  const f = await setup();
  await act(async () => { f.editor.insertBlocks([{ id: "ordinary-new", type: "paragraph", content: "" }], "hbs-formula", "after"); });
  await act(async () => { f.editor.updateBlock("ordinary-new", { content: "1" }); });
  let saving: Promise<void> | undefined;
  if (phase === "before-ack") {
    f.hold(); await act(async () => { saving = f.session.save(); for (let index = 0; index < 30; index++) await Promise.resolve(); });
  } else await act(async () => f.session.save());
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  if (phase === "before-ack") await act(async () => { f.release(); await saving; });
  assertOrdinary(f.session); await act(async () => f.session.preserve());
  const reopened = await HbsSession.open(f.options);
  assertOrdinary(reopened); reopened.dispose();
});

it.each([false, true])("keeps production block moves ordinary through native history and recovery (saved move: %s)", async saveDuringMove => {
  const f = await setup();
  await act(async () => { f.editor.insertBlocks([{ id: "ordinary-new", type: "paragraph", content: "1" }], "hbs-next", "after"); });
  await act(async () => { await f.session.save(); f.editor.transact(transaction => { closeHistory(transaction); }); });
  await act(async () => { expect(moveBlocks(f.editor, ["ordinary-new"], { blockId: "hbs-formula", placement: "after" }, noteBlockMovePolicy)).toBe(true); });
  if (saveDuringMove) await act(async () => f.session.save());
  expect(ids(f.session.getSnapshot().content).indexOf(noteBlockId("ordinary-new"))).toBe(5); assertOrdinary(f.session);
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(ids(f.session.getSnapshot().content).at(-1)).toBe("ordinary-new"); assertOrdinary(f.session);
  await act(async () => {
    expect(f.editor.redo()).toBe(true);
    f.editor.transact(transaction => { closeHistory(transaction); });
    f.editor.updateBlock("ordinary-new", { content: "" });
  });
  await act(async () => f.session.preserve());
  const reopened = await HbsSession.open(f.options);
  assertOrdinary(reopened); reopened.dispose();
});
