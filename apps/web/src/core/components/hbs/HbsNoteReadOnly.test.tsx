// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { HbsSession, createNoteDocumentAdapter, type DocumentSnapshot } from "@research-assistant/core/hbs";
import type { NoteDocumentV1 } from "@research-assistant/core/notes";
import { NotesProvider } from "../../features/notes/NotesClient";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import type { NoteEditorInstallPort } from "../../features/notes/editor/noteEditorInstallPort";
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
async function frame() {
  await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
}
async function setup() {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const adapter = createNoteDocumentAdapter(directTextDiff);
  let saved: DocumentSnapshot<NoteDocumentV1> = { version: 1, saved: noteFixture, baseline: noteFixture };
  const session = await HbsSession.open({ adapter, requestId: n => `readonly-${n}`,
    preservation: { load: () => undefined, save: () => {} },
    port: { read: async () => saved, commit: async request => {
      saved = { version: saved.version + 1, saved: request.saved, baseline: request.baseline }; return saved;
    } } });
  const host = document.createElement("div"), controls = document.createElement("div"); document.body.append(host, controls);
  const root = createRoot(host); let editor: NoteEditorInstance | undefined, port: NoteEditorInstallPort | undefined;
  const render = async (readOnly = false) => {
    await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
      <HbsNoteEditor session={session} isReadOnly={readOnly} diffControlsTarget={controls}
        onEditorReady={value => { editor = value; }} onInstallPort={value => { port = value; }}/>
    </NotesProvider>));
    await frame();
  };
  cleanups.push(async () => { await act(async () => root.unmount()); session.dispose(); host.remove(); controls.remove(); });
  await render();
  return { session, host, controls, render,
    get editor() { if (!editor) throw new Error("missing editor"); return editor; },
    get port() { if (!port) throw new Error("missing port"); return port; },
    external: async () => {
      saved = { ...saved, version: saved.version + 1, saved: adapter.replace(noteFixture,
        ["blocks", { key: "id", value: "hbs-score" }, "content", "items", 0, "text"], "外部候选 91%。") };
      await act(async () => session.syncLatest()); await frame();
    } };
}

it("blocks native undo and redo in a retained readonly note, then restores the same history on activation", async () => {
  const f = await setup(), view = f.editor.prosemirrorView;
  await act(async () => { f.editor.updateBlock("hbs-score", { content: "人工输入" }); });
  const current = f.session.getSnapshot().content;
  await f.render(true);
  expect(f.editor.isEditable).toBe(false);
  expect(view.dom.getAttribute("contenteditable")).toBe("false");
  await act(async () => { f.editor.undo(); f.editor.redo(); });
  for (const key of [{ key: "z", ctrlKey: true }, { key: "z", metaKey: true }, { key: "z", metaKey: true, shiftKey: true }, { key: "y", ctrlKey: true }]) {
    await act(async () => { view.dom.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, cancelable: true, ...key })); });
  }
  expect(f.session.getSnapshot().content).toEqual(current);
  expect(f.port.read()).toEqual(current);
  f.port.setWriteLocked?.(false);
  expect(f.editor.isEditable).toBe(false);
  expect(() => f.port.prepareStructure(current, noteFixture)).toThrow();
  await f.render(false);
  expect(f.editor.prosemirrorView).toBe(view);
  expect(f.editor.isEditable).toBe(true);
  await act(async () => { f.editor.undo(); });
  expect(f.session.getSnapshot().content).toEqual(noteFixture);
  await act(async () => { f.editor.redo(); });
  expect(f.session.getSnapshot().content).toEqual(current);
});

it("removes modifying review and asset entries and disables complete formulas while readonly without accepting candidates", async () => {
  const f = await setup(); await f.external();
  const before = f.session.getSnapshot();
  expect(f.host.querySelector(".note-diff-chunk-actions")).not.toBeNull();
  expect(f.controls.textContent).toContain("Keep");
  await f.render(true);
  expect(f.host.querySelector(".note-diff-chunk-actions")).toBeNull();
  expect(f.controls.querySelector("button")).toBeNull();
  expect(f.host.querySelector('input[type="file"]')).toBeNull();
  const formulas = [...f.host.querySelectorAll<HTMLButtonElement>(".note-formula-disclosure, .inline-equation-preview")];
  expect(formulas).toHaveLength(2);
  for (const button of formulas) {
    expect(button.disabled).toBe(true);
    await act(async () => button.click());
  }
  expect(document.querySelector('[aria-label="编辑公式"], [aria-label="编辑行内公式"]')).toBeNull();
  expect(f.session.getSnapshot().content).toEqual(before.content);
  expect(f.session.getSnapshot().baselineProjection).toEqual(before.baselineProjection);
  expect(f.session.getSnapshot().changes).toEqual(before.changes);
  await f.render(false);
  expect(f.host.querySelector(".note-diff-chunk-actions")).not.toBeNull();
  expect(f.controls.textContent).toContain("Keep");
  expect(f.host.querySelector('input[type="file"]')).not.toBeNull();
  expect(f.host.querySelector<HTMLButtonElement>(".inline-equation-preview")?.disabled).toBe(false);
  expect(f.session.getSnapshot().changes).toEqual(before.changes);
});
