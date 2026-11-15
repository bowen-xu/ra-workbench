// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { undoDepth, redoDepth } from "@tiptap/pm/history";
import { afterEach, expect, it, vi } from "vitest";
import { HbsSession, createNoteDocumentAdapter, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { noteBlockId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { NotesProvider } from "../../features/notes/NotesClient";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import { noteFixture } from "../../../app/previews/hbs-adapters/fixtures";
import { noteRendererGateway } from "../../../app/previews/hbs-adapters/noteGateway";
import { directTextDiff } from "./codeMirrorHbsPolicy";
import { HbsNoteEditor } from "./HbsNoteEditor";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", { configurable: true, value: "CSS1Compat" });
});
const disposals: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const dispose of disposals.splice(0)) await dispose();
  vi.unstubAllGlobals();
});
async function frame() {
  await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
}
async function setup(initial = noteFixture, baseline = initial) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const adapter = createNoteDocumentAdapter(directTextDiff);
  let saved: DocumentSnapshot<NoteDocumentV1> = { version: 1, saved: initial, baseline };
  const session = await HbsSession.open({ adapter, requestId: index => `input-boundary-${index}`,
    preservation: { load: () => undefined, save: () => {} },
    port: { read: async () => saved, commit: async request => {
      expect(request.expectedVersion).toBe(saved.version);
      saved = { version: saved.version + 1, saved: request.saved, baseline: request.baseline };
      return saved;
    } } });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  let editor: NoteEditorInstance | undefined;
  const render = async (readOnly = false) => {
    await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
      <HbsNoteEditor session={session} isReadOnly={readOnly} onEditorReady={value => { editor = value; }}/>
    </NotesProvider>));
    await frame();
  };
  disposals.push(async () => { await act(async () => root.unmount()); session.dispose(); host.remove(); });
  await render();
  const edits = vi.spyOn(session, "edit");
  return { session, adapter, host, render, edits,
    get editor() { if (!editor) throw new Error("missing native editor"); return editor; },
    get saved() { return saved; },
    external: async (content: NoteDocumentV1) => {
      saved = { ...saved, version: saved.version + 1, saved: content };
      await act(async () => session.syncLatest()); await frame();
    } };
}

it("does not republish controlled external block installation as manual input or add a native history item", async () => {
  const f = await setup(), native = f.editor.prosemirrorView;
  const history = [undoDepth(f.editor.prosemirrorState), redoDepth(f.editor.prosemirrorState)];
  const path = ["blocks", { key: "id", value: "hbs-score" }, "content", "items", 0, "text"] as const;
  const changed = f.adapter.replace(noteFixture, path, "外部更新 91%。");
  const blocks = [...changed.blocks], last = blocks.pop()!;
  blocks.splice(1, 0, last);
  const incoming = { ...changed, blocks };
  await f.external(incoming);
  expect(f.edits).not.toHaveBeenCalled();
  expect(f.editor.prosemirrorView).toBe(native);
  expect(f.session.getSnapshot().content).toEqual(incoming);
  expect(f.editor.document.map(block => block.id)).toEqual(incoming.blocks.map(block => block.id));
  expect(f.session.getSnapshot().baselineProjection).toEqual(noteFixture);
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(f.session.getSnapshot().changes.map(change => change.kind).sort()).toEqual(["order", "text"]);
  expect([undoDepth(f.editor.prosemirrorState), redoDepth(f.editor.prosemirrorState)]).toEqual(history);
});

it("publishes one native input exactly once and never reclassifies the save receipt or editability fence as input", async () => {
  const f = await setup(), native = f.editor.prosemirrorView;
  await act(async () => f.editor.setTextCursorPosition("hbs-score", "end"));
  await act(async () => native.dispatch(native.state.tr.insertText("123")));
  expect(f.edits).toHaveBeenCalledTimes(1);
  const typed = f.session.getSnapshot().content;
  expect(f.session.getSnapshot().baselineProjection).toEqual(typed);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  await act(async () => f.session.save()); await frame();
  expect(f.edits).toHaveBeenCalledTimes(1);
  expect(f.saved.saved).toEqual(typed); expect(f.saved.baseline).toEqual(typed);
  const history = [undoDepth(f.editor.prosemirrorState), redoDepth(f.editor.prosemirrorState)];
  await f.render(true); await f.render(false);
  expect(f.editor.prosemirrorView).toBe(native);
  expect(f.edits).toHaveBeenCalledTimes(1);
  expect([undoDepth(f.editor.prosemirrorState), redoDepth(f.editor.prosemirrorState)]).toEqual(history);
  expect(f.session.getSnapshot().content).toEqual(typed);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
});

it("preserves sparse props when controlled updates and native input round-trip the same production schema", async () => {
  const initial = { ...noteFixture, blocks: noteFixture.blocks.map(block => ({ ...block, props: {},
    children: block.children.map(child => ({ ...child, props: {} })) })) };
  const f = await setup(initial);
  const path = ["blocks", { key: "id", value: "hbs-score" }, "content", "items", 0, "text"] as const;
  const changed = f.adapter.replace(initial, path, "外部更新。");
  await f.external(changed);
  expect(f.edits).not.toHaveBeenCalled();
  await act(async () => f.editor.setTextCursorPosition("hbs-next", "end"));
  await act(async () => f.editor.prosemirrorView.dispatch(f.editor.prosemirrorState.tr.insertText("123")));
  expect(f.edits).toHaveBeenCalledTimes(1);
  const actual = f.session.getSnapshot().content;
  expect(actual.blocks.map(block => block.props)).toEqual(changed.blocks.map(block => block.props));
  expect(actual.blocks.flatMap(block => block.children.map(child => child.props))).toEqual(changed.blocks.flatMap(block => block.children.map(child => child.props)));
  expect(f.session.getSnapshot().changes.map(change => change.kind)).toEqual(["text"]);
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
});

const styled: NoteDocumentV1["blocks"][number] = {
  id: noteBlockId("hbs-demo-styled"), type: "paragraph", props: {}, children: [],
  content: { kind: "inline", items: [
    { type: "text", text: "外部修改", styles: ["bold", "italic"], textColor: "green" },
    { type: "link", href: "https://example.org/hbs/194", content: [{ type: "text", text: "合成链接", styles: ["underline"] }] },
    { type: "mention", targetKind: "paper", targetId: "synthetic-paper", labelSnapshot: "合成文献提及" },
  ] },
};
it("does not introduce default-prop review changes in a fresh rich paragraph matching the reported block", async () => {
  const initial = { ...noteFixture, blocks: [...noteFixture.blocks, styled] };
  const f = await setup(initial);
  await act(async () => f.editor.setTextCursorPosition("hbs-next", "end"));
  await act(async () => f.editor.prosemirrorView.dispatch(f.editor.prosemirrorState.tr.insertText("123")));
  await act(async () => f.session.save()); await frame();
  expect(f.session.getSnapshot().content.blocks.at(-1)).toEqual(styled);
  expect(f.saved.saved.blocks.at(-1)).toEqual(styled);
  expect(f.saved.baseline.blocks.at(-1)).toEqual(styled);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
});

it("does not silently erase already persisted default-prop differences when opening and typing elsewhere", async () => {
  const baseline = { ...noteFixture, blocks: [...noteFixture.blocks, styled] };
  const present = { ...styled, props: { backgroundColor: "default", textAlignment: "left", textColor: "default" } };
  const initial = { ...noteFixture, blocks: [...noteFixture.blocks, present] };
  const f = await setup(initial, baseline);
  const pending = f.session.getSnapshot().changes;
  expect(pending).toHaveLength(3);
  expect(pending.every(change => change.kind === "property")).toBe(true);
  expect(f.edits).not.toHaveBeenCalled();
  await act(async () => f.editor.setTextCursorPosition("hbs-next", "end"));
  await act(async () => f.editor.prosemirrorView.dispatch(f.editor.prosemirrorState.tr.insertText("123")));
  await act(async () => f.session.save()); await frame();
  expect(f.session.getSnapshot().changes).toEqual(pending);
  expect(f.saved.saved.blocks.at(-1)).toEqual(present);
  expect(f.saved.baseline.blocks.at(-1)).toEqual(styled);
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
});
