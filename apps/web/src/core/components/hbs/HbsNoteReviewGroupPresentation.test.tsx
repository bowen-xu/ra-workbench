// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, expect, it, vi } from "vitest";
import { HbsSession, createNoteDocumentAdapter, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { noteBlockId, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";
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
  vi.unstubAllGlobals();
});
function paragraph(id: string, text: string, type: NoteBlockV1["type"] = "paragraph"): NoteBlockV1 {
  return {
    id: noteBlockId(id), type, props: { textColor: "default", backgroundColor: "default", textAlignment: "left" },
    content: { kind: "inline", items: [{ type: "text", text, styles: [] }] }, children: [],
  };
}
const sample = (blocks: readonly NoteBlockV1[]): NoteDocumentV1 => ({ ...noteFixture, blocks });
const textPath = (id: string) => ["blocks", { key: "id", value: id }, "content", "items", 0, "text"] as const;
const snapshotTexts = (snapshot: HTMLElement): string[] => [...(snapshot.querySelector(".bn-block-group")?.children ?? [])].map(block => block.textContent ?? "");
function precedes(first: Node, second: Node): void {
  expect(first.compareDocumentPosition(second) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
}
function formulas(root: HTMLElement): string[] {
  return [...root.querySelectorAll('annotation[encoding="application/x-tex"]')].map(element => element.textContent ?? "");
}

async function setup(initial: NoteDocumentV1) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const adapter = createNoteDocumentAdapter(directTextDiff);
  let saved: DocumentSnapshot<NoteDocumentV1> = { version: 1, saved: initial, baseline: initial };
  let editor: NoteEditorInstance | undefined;
  const session = await HbsSession.open({
    adapter, requestId: sequence => `group-presentation-${sequence}`, preservation: { load: () => undefined, save: () => {} },
    port: {
      read: async () => saved,
      commit: async request => {
        expect(request.expectedVersion).toBe(saved.version);
        saved = { version: saved.version + 1, saved: request.saved, baseline: request.baseline };
        return saved;
      },
    },
  });
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  cleanups.push(async () => { await act(async () => root.unmount()); session.dispose(); host.remove(); });
  await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={session} onEditorReady={value => { editor = value; }}/>
  </NotesProvider>));
  await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
  return {
    adapter, session, host,
    get saved() { return saved; },
    get editor() { if (!editor) throw new Error("editor not mounted"); return editor; },
    old: () => host.querySelector<HTMLElement>(".note-diff-deleted-blocks[data-hbs-review-group]:not(.hbs-note-incoming)")!,
    incoming: () => host.querySelector<HTMLElement>(".hbs-note-incoming[data-hbs-review-group]")!,
    current: () => [...host.querySelectorAll<HTMLElement>(".note-diff-added-block[data-hbs-review-group][data-id]")],
    actions: () => [...host.querySelectorAll<HTMLElement>("[data-hbs-review-decisions]")],
    external: async (document: NoteDocumentV1) => {
      saved = { ...saved, version: saved.version + 1, saved: document };
      await act(async () => session.syncLatest());
    },
  };
}

it("renders the complete old run before both native current paragraphs with one action row", async () => {
  const initial = sample([paragraph("a", "Old A 80"), paragraph("b", "Old B 80"), paragraph("tail", "Stable tail")]);
  const f = await setup(initial), native = f.editor.prosemirrorView;
  const incoming = f.adapter.replace(f.adapter.replace(initial, textPath("a"), "Current A 85"), textPath("b"), "Current B 85");
  await f.external(incoming);

  expect(f.host.querySelectorAll(".note-diff-deleted-blocks")).toHaveLength(1);
  expect(snapshotTexts(f.old())).toEqual(["Old A 80", "Old B 80"]);
  expect(f.current().map(block => block.dataset.id)).toEqual(["a", "b"]);
  expect(f.actions()).toHaveLength(1);
  precedes(f.old(), f.current()[0]!);
  precedes(f.current()[0]!, f.current()[1]!);
  precedes(f.current()[1]!, f.actions()[0]!);
  expect(new Set([f.old(), ...f.current()].map(element => element.dataset.hbsReviewGroup)).size).toBe(1);
  expect(f.current()[0]!.classList.contains("hbs-note-group-continuation")).toBe(false);
  expect(f.current()[1]!.classList.contains("hbs-note-group-continuation")).toBe(true);
  expect(f.old().getAttribute("contenteditable")).toBe("false");
  expect(f.old().querySelector("[data-id],[data-node-type=blockContainer]")).toBeNull();
  expect(f.host.querySelectorAll('[data-node-type="blockContainer"][data-id="a"]')).toHaveLength(1);
  expect(f.host.querySelectorAll('[data-node-type="blockContainer"][data-id="b"]')).toHaveLength(1);
  expect(f.editor.prosemirrorView).toBe(native);
  expect(f.session.getSnapshot().content).toEqual(incoming);
  expect(f.saved.baseline).toEqual(initial);
});

it("keeps old block and inline formulas together ahead of both current formula blocks", async () => {
  const blockFormula = (source: string): NoteBlockV1 => ({ ...paragraph("block-formula", ""), type: "formulaBlock", props: {}, content: { kind: "formula", source } });
  const inlineFormula = (source: string): NoteBlockV1 => ({ ...paragraph("inline-formula", ""), content: { kind: "inline", items: [
    { type: "text", text: "Inline formula: ", styles: [] }, { type: "inlineFormula", source },
  ] } });
  const initial = sample([blockFormula("E = mc^2"), inlineFormula("x^2"), paragraph("tail", "Stable")]);
  const f = await setup(initial);
  await f.external(sample([blockFormula("E = 2mc^2"), inlineFormula("x^3"), initial.blocks[2]!]));

  expect(f.host.querySelectorAll(".note-diff-deleted-blocks")).toHaveLength(1);
  expect(formulas(f.old())).toEqual(["E = mc^2", "x^2"]);
  expect(f.current().flatMap(formulas)).toEqual(["E = 2mc^2", "x^3"]);
  precedes(f.old(), f.current()[0]!);
  expect(f.old().querySelectorAll(".katex")).toHaveLength(2);
  expect(f.current()[1]?.classList.contains("hbs-note-group-continuation")).toBe(true);
  expect(f.actions()).toHaveLength(1);
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
});

it("serializes a code block and quote as separate rich old blocks in the same snapshot", async () => {
  const code = (source: string): NoteBlockV1 => ({ ...paragraph("code", source, "codeBlock"), props: { language: "python" } });
  const quote = (text: string): NoteBlockV1 => ({ ...paragraph("quote", text, "quote"), content: { kind: "inline", items: [
    { type: "text", text, styles: ["bold"] },
  ] } });
  const initial = sample([code("score = 80\nprint(score)"), quote("Old quote 80"), paragraph("tail", "Stable")]);
  const f = await setup(initial);
  await f.external(sample([code("score = 85\nprint(score)"), quote("New quote 85"), initial.blocks[2]!]));

  expect(f.host.querySelectorAll(".note-diff-deleted-blocks")).toHaveLength(1);
  expect(f.old().querySelector("pre > code")?.textContent).toBe("score = 80\nprint(score)");
  expect(f.old().querySelector("blockquote")?.textContent).toBe("Old quote 80");
  expect(f.old().querySelector("blockquote strong,blockquote b")?.textContent).toBe("Old quote 80");
  expect(f.old().querySelectorAll("pre > code")).toHaveLength(1);
  expect(f.old().querySelectorAll("blockquote")).toHaveLength(1);
  expect(f.old().querySelector(".note-diff-deleted-codeBlock,.note-diff-deleted-quote")).toBeNull();
  expect(f.current().map(block => block.dataset.id)).toEqual(["code", "quote"]);
  expect(f.current()[0]!.querySelector("pre > code")?.textContent).toBe("score = 85\nprint(score)");
  precedes(f.old(), f.current()[0]!);
  expect(f.actions()).toHaveLength(1);
});

it("renders all old then all incoming then all editable current content for adjacent conflicts", async () => {
  const initial = sample([paragraph("a", "Score A 80"), paragraph("b", "Score B 80"), paragraph("tail", "Stable")]);
  const f = await setup(initial), native = f.editor.prosemirrorView;
  const candidates = ["a", "b"].reduce((document, id) => f.adapter.replace(document, textPath(id), `Score ${id.toUpperCase()} 85`), initial);
  await f.external(candidates);
  await act(async () => {
    f.editor.updateBlock("a", { content: "Score A 86" });
    f.editor.updateBlock("b", { content: "Score B 86" });
  });
  await f.external(["a", "b"].reduce((document, id) => f.adapter.replace(document, textPath(id), `Score ${id.toUpperCase()} 87`), candidates));

  expect(f.session.getSnapshot().conflicts).toHaveLength(2);
  expect(f.host.querySelectorAll(".note-diff-deleted-blocks:not(.hbs-note-incoming)")).toHaveLength(1);
  expect(f.host.querySelectorAll(".hbs-note-incoming")).toHaveLength(1);
  expect(snapshotTexts(f.old())).toEqual(["Score A 80", "Score B 80"]);
  expect(snapshotTexts(f.incoming())).toEqual(["Score A 87", "Score B 87"]);
  expect(f.current().map(block => block.textContent)).toEqual(["Score A 86", "Score B 86"]);
  precedes(f.old(), f.incoming());
  precedes(f.incoming(), f.current()[0]!);
  precedes(f.current()[1]!, f.actions()[0]!);
  expect(f.incoming().getAttribute("contenteditable")).toBe("false");
  expect(f.incoming().querySelector("[data-id],[data-node-type=blockContainer]")).toBeNull();
  expect(new Set([f.old(), f.incoming(), ...f.current()].map(element => element.dataset.hbsReviewGroup)).size).toBe(1);
  expect(f.actions()).toHaveLength(1);
  expect([...f.actions()[0]!.querySelectorAll("button")].map(button => button.textContent)).toEqual(["Undo", "Take", "Both", "Keep"]);
  expect(f.editor.prosemirrorView).toBe(native);
});

it("places a deleted and modified old run before replacement and modified current blocks", async () => {
  const initial = sample([paragraph("removed", "Deleted old"), paragraph("changed", "Modified old 80"), paragraph("tail", "Stable tail")]);
  const f = await setup(initial);
  const incoming = sample([paragraph("replacement", "Replacement new"), paragraph("changed", "Modified new 85"), initial.blocks[2]!]);
  await f.external(incoming);

  expect(f.session.getSnapshot().changes).toHaveLength(3);
  expect(snapshotTexts(f.old())).toEqual(["Deleted old", "Modified old 80"]);
  expect(f.current().map(block => block.dataset.id)).toEqual(["replacement", "changed"]);
  precedes(f.old(), f.current()[0]!);
  precedes(f.current()[1]!, f.actions()[0]!);
  const tail = f.host.querySelector('[data-node-type="blockContainer"][data-id="tail"]')!;
  precedes(f.actions()[0]!, tail);
  expect(tail.classList.contains("note-diff-added-block")).toBe(false);
  expect(f.actions()).toHaveLength(1);
  expect(f.old().querySelector('[data-id="removed"]')).toBeNull();
});

it("updates only the second current member during input while its grouped old snapshot stays unchanged", async () => {
  const initial = sample([paragraph("a", "Score A 80"), paragraph("b", "Score B 80"), paragraph("tail", "Stable")]);
  const f = await setup(initial), native = f.editor.prosemirrorView;
  const incoming = f.adapter.replace(f.adapter.replace(initial, textPath("a"), "Score A 85"), textPath("b"), "Score B 85");
  await f.external(incoming);
  const oldHtml = f.old().innerHTML, saved = f.saved;

  await act(async () => f.editor.updateBlock("b", { content: "Score B 86" }));

  expect(f.old().innerHTML).toBe(oldHtml);
  expect(snapshotTexts(f.old())).toEqual(["Score A 80", "Score B 80"]);
  expect(f.adapter.read(f.session.getSnapshot().content, textPath("a"))).toBe("Score A 85");
  expect(f.adapter.read(f.session.getSnapshot().content, textPath("b"))).toBe("Score B 86");
  expect(f.saved).toBe(saved);
  expect(f.saved.saved).toEqual(incoming);
  expect(f.saved.baseline).toEqual(initial);
  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(f.session.getSnapshot().changes).toHaveLength(2);
  expect(f.current()[1]!.textContent).toBe("Score B 86");
  expect(f.current()[1]!.classList.contains("hbs-note-group-continuation")).toBe(true);
  expect(f.actions()).toHaveLength(1);
  expect(f.host.querySelectorAll(".bn-editor")).toHaveLength(1);
  expect(f.editor.prosemirrorView).toBe(native);
});

it("shows an explicit readonly incoming deletion for the entire conflicted run before Take", async () => {
  const initial = sample([paragraph("a", "A80"), paragraph("b", "B80"), paragraph("c", "Stable C")]);
  const f = await setup(initial);
  await f.external(f.adapter.replace(f.adapter.replace(initial, textPath("a"), "A85"), textPath("b"), "B85"));
  await act(async () => {
    f.editor.updateBlock("a", { content: "A86" });
    f.editor.updateBlock("b", { content: "B86" });
  });
  expect(f.adapter.read(f.session.getSnapshot().content, textPath("a"))).toBe("A86");
  expect(f.adapter.read(f.session.getSnapshot().content, textPath("b"))).toBe("B86");
  await f.external(sample([initial.blocks[2]!]));
  const state = f.session.getSnapshot();
  expect(state.conflicts).toHaveLength(2);
  expect(state.conflicts.map(conflict => conflict.path)).toEqual([
    ["blocks", { key: "id", value: "a" }], ["blocks", { key: "id", value: "b" }],
  ]);
  expect(state.conflicts.every(conflict => conflict.incoming === undefined)).toBe(true);
  expect(f.host.querySelector(".hbs-review-projection-reason")).toBeNull();
  expect(f.host.querySelectorAll(".note-diff-deleted-blocks:not(.hbs-note-incoming)")).toHaveLength(1);
  expect(f.host.querySelectorAll(".hbs-note-incoming")).toHaveLength(1);
  expect(f.incoming().textContent).toBe("此范围在 incoming 中已删除。");
  expect(f.incoming().getAttribute("contenteditable")).toBe("false");
  expect(f.current().map(block => block.textContent)).toEqual(["A86", "B86"]);
  expect(f.actions()).toHaveLength(1);
  precedes(f.old(), f.incoming());
  precedes(f.incoming(), f.current()[0]!);
  precedes(f.current()[1]!, f.actions()[0]!);
  const observed = f.saved;
  const take = [...f.actions()[0]!.querySelectorAll("button")].find(button => button.textContent === "Take");
  if (!take) throw new Error("missing grouped deletion Take");

  await act(async () => take.click());

  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(f.session.getSnapshot().content.blocks.map(block => block.id)).toEqual(["c"]);
  expect(f.session.getSnapshot().changes).toHaveLength(2);
  expect(f.saved).toBe(observed);
  expect(f.saved.baseline).toEqual(initial);
});
