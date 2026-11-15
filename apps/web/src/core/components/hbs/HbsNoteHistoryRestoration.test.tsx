// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { closeHistory } from "@tiptap/pm/history";
import { afterEach, expect, it, vi } from "vitest";
import { HbsSession, createNoteDocumentAdapter, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { noteBlockId, notePageId, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { NotesProvider } from "../../features/notes/NotesClient";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import { noteRendererGateway } from "../../../app/previews/hbs-adapters/noteGateway";
import { directTextDiff } from "./codeMirrorHbsPolicy";
import { HbsNoteEditor } from "./HbsNoteEditor";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", { configurable: true, value: "CSS1Compat" });
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) await cleanup();
  vi.unstubAllGlobals();
});
function paragraph(id: string, text = "", children: readonly NoteBlockV1[] = []): NoteBlockV1 {
  return { id: noteBlockId(id), type: "paragraph", props: {}, children,
    content: { kind: "inline", items: text ? [{ type: "text", text, styles: [] }] : [] } };
}
function documentWith(...blocks: NoteBlockV1[]): NoteDocumentV1 {
  return { schemaVersion: 1, pageId: notePageId("history-restoration"), blocks };
}
async function frame() {
  await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
}
async function setup(initial: NoteDocumentV1, toolbar: "external" | "embedded" | "hidden" = "external") {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const adapter = createNoteDocumentAdapter(directTextDiff);
  let saved: DocumentSnapshot<NoteDocumentV1> = { version: 1, saved: initial, baseline: initial };
  const session = await HbsSession.open({ adapter, requestId: sequence => `restoration-${sequence}`,
    preservation: { load: () => undefined, save: () => {} },
    port: { read: async () => saved, commit: async request => {
      expect(request.expectedVersion).toBe(saved.version);
      saved = { version: saved.version + 1, saved: request.saved, baseline: request.baseline }; return saved;
    } } });
  const host = document.createElement("div"), controls = document.createElement("div");
  document.body.append(host, controls);
  const root = createRoot(host);
  let editor: NoteEditorInstance | undefined;
  cleanups.push(async () => { await act(async () => root.unmount()); session.dispose(); host.remove(); controls.remove(); });
  await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={session} {...(toolbar === "embedded" ? {} : { diffControlsTarget: toolbar === "external" ? controls : null })}
      onEditorReady={value => { editor = value; }}/>
  </NotesProvider>));
  await frame();
  return { session, adapter, host, controls,
    get editor() { if (!editor) throw new Error("editor not mounted"); return editor; },
    get saved() { return saved; },
    external: async (content: NoteDocumentV1) => {
      saved = { ...saved, version: saved.version + 1, saved: content };
      await act(async () => session.syncLatest()); await frame();
    },
    click: async (label: string) => {
      const button = [...host.querySelectorAll("button"), ...controls.querySelectorAll("button")]
        .find(button => button.textContent === label || button.getAttribute("aria-label") === label);
      if (!button) throw new Error(`missing ${label}`);
      await act(async () => button.click()); await frame();
    } };
}
function expectClean(session: HbsSession<NoteDocumentV1>) {
  expect(session.getSnapshot().changes).toHaveLength(0);
  expect(session.getSnapshot().conflicts).toHaveLength(0);
}

it.each([false, true])("restores sparse empty, text and nested blocks without review differences (saved deletion: %s)", async saveDeletion => {
  const removed = { ...paragraph("removed", "", [paragraph("nested", "Nested text")]), props: { syntheticMetadata: "preserved" } };
  const initial = documentWith(paragraph("body", "Stable"), removed, paragraph("text", "Restored text"));
  const f = await setup(initial), native = f.editor.prosemirrorView;
  await act(async () => { f.editor.removeBlocks(["removed", "text"]); });
  if (saveDeletion) await act(async () => f.session.save());
  expectClean(f.session);
  await act(async () => { expect(f.editor.undo()).toBe(true); }); await frame();
  expect(f.session.getSnapshot().content).toEqual(initial);
  expect(f.session.getSnapshot().baselineProjection).toEqual(initial);
  expectClean(f.session); expect(f.controls.textContent).toBe("");
  await act(async () => f.session.save());
  expect(f.saved.saved).toEqual(initial); expect(f.saved.baseline).toEqual(initial);
  await act(async () => { expect(f.editor.redo()).toBe(true); }); await frame();
  expect(f.session.getSnapshot().content.blocks.map(block => block.id)).toEqual(["body"]);
  expectClean(f.session); expect(f.editor.prosemirrorView).toBe(native);
});

it("retains explicit color/alignment edits and their original sparse representation through history", async () => {
  const initial = documentWith(paragraph("body", "Style"));
  const f = await setup(initial);
  await act(async () => f.editor.updateBlock("body", { props: { textColor: "red", textAlignment: "right" } }));
  await act(async () => f.session.save());
  expect(f.saved.saved.blocks[0]!.props).toMatchObject({ textColor: "red", textAlignment: "right" });
  await act(async () => { expect(f.editor.undo()).toBe(true); }); await frame();
  expect(f.session.getSnapshot().content).toEqual(initial); expectClean(f.session);
  await act(async () => { expect(f.editor.redo()).toBe(true); }); await frame();
  expect(f.session.getSnapshot().content.blocks[0]!.props).toMatchObject({ textColor: "red", textAlignment: "right" });
  expectClean(f.session);
});

it("restores a removed sparse block without accepting a later unrelated external edit", async () => {
  const empty = paragraph("empty"), initial = documentWith(paragraph("body", "Original"), empty);
  const f = await setup(initial);
  await act(async () => f.editor.removeBlocks(["empty"]));
  await act(async () => f.session.save());
  const incoming = documentWith(paragraph("body", "External update"));
  await f.external(incoming);
  await act(async () => { expect(f.editor.undo()).toBe(true); }); await frame();
  expect(f.session.getSnapshot().content.blocks).toEqual([...incoming.blocks, empty]);
  expect(f.session.getSnapshot().baselineProjection).toEqual(initial);
  expect(f.session.getSnapshot().changes.map(change => change.kind)).toEqual(["text"]);
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
});

it("restores sparse properties across separate native typing and deletion entries", async () => {
  const initial = documentWith(paragraph("body", "Stable"), paragraph("empty"));
  const f = await setup(initial);
  await act(async () => {
    f.editor.transact(transaction => { closeHistory(transaction); });
    f.editor.setTextCursorPosition("body", "end");
    f.editor.prosemirrorView.dispatch(f.editor.prosemirrorState.tr.insertText(" input"));
    f.editor.removeBlocks(["empty"]);
  });
  await act(async () => f.session.save());
  await act(async () => { expect(f.editor.undo()).toBe(true); }); await frame();
  expect(f.session.getSnapshot().content).toEqual(documentWith(paragraph("body", "Stable input"), paragraph("empty")));
  expectClean(f.session);
  await act(async () => { expect(f.editor.undo()).toBe(true); }); await frame();
  expect(f.session.getSnapshot().content).toEqual(initial); expectClean(f.session);
});

it.each(["保留第 1 处变更", "撤销第 1 处变更", "Keep all", "Undo all"])("removes the final review controls after %s and restores them with native Undo", async label => {
  const initial = documentWith(paragraph("body", "Original"));
  const f = await setup(initial), incoming = documentWith(paragraph("body", "Incoming"));
  await f.external(incoming);
  expect(f.controls.textContent).toContain("1/1");
  await f.click(label);
  expectClean(f.session); expect(f.session.getSnapshot().pending).toBe(false);
  expect(f.host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(0);
  expect(f.controls.querySelector(".note-editor-diff-controls")).toBeNull();
  await act(async () => { expect(f.editor.undo()).toBe(true); }); await frame();
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.controls.textContent).toContain("1/1");
  await act(async () => { expect(f.editor.redo()).toBe(true); }); await frame();
  expectClean(f.session); expect(f.controls.textContent).toBe("");
});

it("prunes removed groups while retaining an unchanged group's controls and navigation", async () => {
  const initial = documentWith(paragraph("first", "Old first"), paragraph("stable", "Unchanged"), paragraph("last", "Old last"));
  const f = await setup(initial);
  await f.external(documentWith(paragraph("first", "New first"), initial.blocks[1]!, paragraph("last", "New last")));
  expect(f.controls.textContent).toContain("1/2");
  const firstActions = f.host.querySelector(".note-diff-chunk-actions");
  await f.click("下一处变更");
  expect(f.controls.textContent).toContain("2/2");
  await f.click("应用当前变更");
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.controls.textContent).toContain("1/1");
  expect(f.host.querySelector(".note-diff-chunk-actions")).toBe(firstActions);
  await f.click("应用当前变更");
  expectClean(f.session); expect(f.controls.textContent).toBe("");
});

it("uses the shared review controls inside a standalone note editor and restores them through native history", async () => {
  const initial = documentWith(paragraph("body", "Original")), f = await setup(initial, "embedded");
  await f.external(documentWith(paragraph("body", "Incoming")));
  const controls = () => f.host.querySelector(".note-editor-surface .note-editor-diff-controls");
  expect(controls()?.textContent).toContain("1/1");
  expect(f.controls.textContent).toBe("");
  await f.click("Keep all"); expectClean(f.session); expect(controls()).toBeNull();
  await act(async () => { expect(f.editor.undo()).toBe(true); }); await frame();
  expect(controls()?.textContent).toContain("1/1");
  await act(async () => { expect(f.editor.redo()).toBe(true); }); await frame();
  expectClean(f.session); expect(controls()).toBeNull();
});

it("keeps global review controls suppressed when a retained host explicitly passes null", async () => {
  const f = await setup(documentWith(paragraph("body", "Original")), "hidden");
  await f.external(documentWith(paragraph("body", "Incoming")));
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.host.querySelector(".note-editor-diff-controls")).toBeNull();
  expect(f.controls.textContent).toBe("");
  expect(f.host.querySelector(".note-diff-chunk-actions")).not.toBeNull();
});
