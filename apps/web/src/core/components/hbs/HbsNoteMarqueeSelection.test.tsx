// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { undoDepth } from "@tiptap/pm/history";
import { afterEach, expect, it, vi } from "vitest";
import { HbsSession, createNoteDocumentAdapter, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { noteBlockId, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { BlockSetSelection } from "../block-editor/BlockSetSelection";
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
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) await cleanup();
  vi.restoreAllMocks(); vi.unstubAllGlobals();
});
function paragraph(id: string, text: string, children: readonly NoteBlockV1[] = []): NoteBlockV1 {
  return {
    id: noteBlockId(id), type: "paragraph", props: { textColor: "default", backgroundColor: "default", textAlignment: "left" },
    content: { kind: "inline", items: [{ type: "text", text, styles: [] }] }, children,
  };
}
function documentWithChildren(value: string): NoteDocumentV1 {
  return { ...noteFixture, blocks: [paragraph("parent", "Parent", [
    paragraph("child-a", `Child A ${value}`), paragraph("child-b", `Child B ${value}`),
  ]), paragraph("peer", "Stable peer")] };
}
function bounds(element: Element, left: number, top: number, width = 500, height = 26): void {
  vi.spyOn(element, "getBoundingClientRect").mockReturnValue(new DOMRect(left, top, width, height));
}
const pointer = (type: "pointerdown" | "pointerup", x: number, y: number): PointerEvent => new PointerEvent(type, {
  bubbles: true, cancelable: true, pointerId: 1, isPrimary: true, pointerType: "mouse", button: 0,
  buttons: type === "pointerup" ? 0 : 1, clientX: x, clientY: y,
});

async function setup(conflict: boolean) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const adapter = createNoteDocumentAdapter(directTextDiff), initial = documentWithChildren("80");
  let saved: DocumentSnapshot<NoteDocumentV1> = { version: 1, saved: initial, baseline: initial };
  let editor: NoteEditorInstance | undefined;
  const session = await HbsSession.open({
    adapter, requestId: sequence => `note-marquee-${sequence}`, preservation: { load: () => undefined, save: () => {} },
    port: {
      read: async () => saved,
      commit: async request => {
        expect(request.expectedVersion).toBe(saved.version);
        saved = { version: saved.version + 1, saved: request.saved, baseline: request.baseline }; return saved;
      },
    },
  });
  const host = document.createElement("div"); host.dataset.blockEditorScrollContainer = "";
  document.body.append(host); const root = createRoot(host);
  cleanups.push(async () => { await act(async () => root.unmount()); session.dispose(); host.remove(); });
  await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={session} onEditorReady={value => { editor = value; }}/>
  </NotesProvider>));
  await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
  const mounted = (() => {
    if (!editor) throw new Error("editor not mounted");
    return editor;
  })();
  const external = async (document: NoteDocumentV1) => {
    saved = { ...saved, version: saved.version + 1, saved: document }; await act(async () => session.syncLatest());
  };
  await external(documentWithChildren("85"));
  if (conflict) {
    await act(async () => {
      mounted.updateBlock("child-a", { content: "Child A 86" });
      mounted.updateBlock("child-b", { content: "Child B 86" });
    });
    await external(documentWithChildren("87"));
  }
  const native = mounted.domElement!;
  const parent = native.querySelector<HTMLElement>('[data-node-type="blockContainer"][data-id="parent"]')!;
  const old = native.querySelector<HTMLElement>(".note-diff-deleted-blocks:not(.hbs-note-incoming)")!;
  const incoming = native.querySelector<HTMLElement>(".hbs-note-incoming");
  expect(old.closest('[data-id="parent"]')).toBe(parent);
  expect(old.querySelectorAll(".bn-block-content")).toHaveLength(2);
  expect(old.querySelector("[data-id]")).toBeNull();
  expect(old.querySelector(".bn-block-content")?.closest(".bn-block[data-id]")).toBe(parent);
  expect(old.getAttribute("contenteditable")).toBe("false");
  bounds(host, 20, 50, 680, 450); bounds(native, 60, 100, 600, 600);
  bounds(native.closest(".block-editor")!, 60, 100, 600, 600);
  bounds(parent.querySelector(".bn-block-content")!, 110, 120);
  old.querySelectorAll(".bn-block-content").forEach((row, index) => bounds(row, 134, 150 + index * 30, 476));
  incoming?.querySelectorAll(".bn-block-content").forEach((row, index) => bounds(row, 134, 210 + index * 30, 476));
  for (const [index, id] of ["child-a", "child-b"].entries()) {
    bounds(native.querySelector(`[data-node-type="blockContainer"][data-id="${id}"] > .bn-block-content`)!, 134, 280 + index * 30, 476);
  }
  bounds(native.querySelector('[data-node-type="blockContainer"][data-id="peer"] > .bn-block-content')!, 110, 350);
  vi.spyOn(window, "requestAnimationFrame").mockReturnValue(1);
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});
  let captured = false;
  native.setPointerCapture = () => { captured = true; };
  native.hasPointerCapture = () => captured;
  native.releasePointerCapture = () => { captured = false; };
  return {
    host, editor: mounted, session, old, incoming,
    get saved() { return saved; },
    select: async (from: number, to: number) => {
      await act(async () => {
        host.dispatchEvent(pointer("pointerdown", 80, from));
        window.dispatchEvent(pointer("pointerup", 450, to));
      });
      const selection = mounted.prosemirrorState.selection;
      return selection instanceof BlockSetSelection ? selection.blockIds : [];
    },
  };
}

it.each([false, true])("selects only native nested children across serialized HBS snapshots (incoming: %s)", async conflict => {
  const f = await setup(conflict), native = f.editor.prosemirrorView;
  const content = f.session.getSnapshot().content, observed = f.saved, history = undoDepth(f.editor.prosemirrorState);
  expect(f.session.getSnapshot().conflicts).toHaveLength(conflict ? 2 : 0);

  expect(await f.select(149, 209)).toEqual([]);
  expect(f.editor.prosemirrorState.selection).not.toBeInstanceOf(BlockSetSelection);
  if (conflict) {
    expect(f.incoming?.closest('[data-id="parent"]')).toBe(f.old.closest('[data-id="parent"]'));
    expect(f.incoming?.querySelectorAll(".bn-block-content")).toHaveLength(2);
    expect(f.incoming?.querySelector("[data-id]")).toBeNull();
    expect(f.incoming?.getAttribute("contenteditable")).toBe("false");
    expect(await f.select(209, 269)).toEqual([]);
  }
  expect(await f.select(279, 309)).toEqual(["child-a"]);
  expect(f.editor.prosemirrorState.selection).toBeInstanceOf(BlockSetSelection);
  expect(await f.select(149, 339)).toEqual(["child-a", "child-b"]);
  expect(f.editor.prosemirrorState.selection).toBeInstanceOf(BlockSetSelection);
  expect(f.editor.prosemirrorView).toBe(native);
  expect(f.session.getSnapshot().content).toEqual(content);
  expect(f.saved).toBe(observed);
  expect(undoDepth(f.editor.prosemirrorState)).toBe(history);
  expect(f.host.querySelectorAll(".note-block-selection-rect")).toHaveLength(0);
});
