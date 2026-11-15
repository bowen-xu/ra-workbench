// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { NoteEditor } from "./NoteEditor";
import type { NoteEditorInstallPort } from "./noteEditorInstallPort";

vi.mock("../NotesClient", async () => {
  const { NoteCommandHistory } = await import("./NoteCommandHistory");
  const history = new NoteCommandHistory();
  return { useNotesClient: () => ({ gateway: {}, history }), useNotesVersion: () => 0 };
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => { for (const cleanup of cleanups.splice(0)) await cleanup(); vi.unstubAllGlobals(); });

it("shows a block-relative source selection after the real editor mounts without changing its document", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  cleanups.push(async () => { await act(async () => root.unmount()); host.remove(); });
  const page: NoteDocumentV1 = {schemaVersion: 1, pageId: notePageId("source"), blocks: [{
    id: noteBlockId("block"), type: "paragraph", props: {}, children: [], content: {kind: "inline", items: [{type: "text", text: "first\n😀second", styles: []}]},
  }]};
  const changed = vi.fn();
  await act(async () => root.render(<NoteEditor document={page} onChange={changed} sourceNavigation={{pageId: "source", blockId: "block", token: 1, range: {startLine: 2, startColumn: 1, endLine: 2, endColumn: 4}}} />));
  await act(async () => {await new Promise<void>(resolve => requestAnimationFrame(() => resolve()));});
  expect(document.activeElement?.closest(".bn-editor")).not.toBeNull();
  expect(window.getSelection()?.toString()).toBe("😀sec");
  expect(changed).not.toHaveBeenCalled();
});

it("publishes a session port, gates IME/read-only and suppresses install save echoes through the real React view", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  cleanups.push(async () => { await act(async () => root.unmount()); host.remove(); });
  const page: NoteDocumentV1 = { schemaVersion: 1, pageId: notePageId("one"), blocks: [{
    id: noteBlockId("first"), type: "paragraph", props: {}, children: [],
    content: { kind: "inline", items: [{ type: "text", text: "Before", styles: [] }] },
  }] };
  const next: NoteDocumentV1 = { ...page, blocks: page.blocks.map(block => ({ ...block,
    content: { kind: "inline", items: [{ type: "text", text: "After", styles: [] }] },
  })) };
  let current: NoteEditorInstallPort | undefined;
  const register = (port: NoteEditorInstallPort | undefined) => { current = port; };
  const onChange = vi.fn();
  const render = (value: NoteDocumentV1, isReadOnly = false) => root.render(<StrictMode>
    <NoteEditor document={value} onChange={onChange} onInstallPort={register} isReadOnly={isReadOnly} />
  </StrictMode>);
  await act(async () => render(page));
  const port = current;
  if (!port) throw new Error("Missing installation port");
  const editorDom = host.querySelector(".bn-editor");
  if (!editorDom) throw new Error("Missing editor DOM");
  await act(async () => { editorDom.dispatchEvent(new CompositionEvent("compositionstart", { bubbles: true })); });
  expect(() => port.prepare(page, next)).toThrow();
  await act(async () => { editorDom.dispatchEvent(new CompositionEvent("compositionend", { bubbles: true })); });
  onChange.mockClear();
  await act(async () => { port.prepare(page, next)(); });
  expect(host.textContent).toContain("After");
  expect(onChange).not.toHaveBeenCalled();
  await act(async () => render(next));
  expect(current).toBe(port);
  expect(host.querySelector(".bn-editor")).toBe(editorDom);
  await act(async () => render(next, true));
  expect(() => port.prepare(next, page)).toThrow();
  await act(async () => root.render(<div />));
  expect(current).toBeUndefined();
  expect(() => port.prepare(next, page)).toThrow();
});
