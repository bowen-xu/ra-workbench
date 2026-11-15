// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { closeHistory, undoDepth } from "@tiptap/pm/history";
import { afterEach, expect, it, vi } from "vitest";
import { HbsSession, createNoteDocumentAdapter, type DocumentSnapshot } from "@research-assistant/core/hbs";
import { noteBlockId, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { NotesProvider } from "../../features/notes/NotesClient";
import type { NoteEditorInstance } from "../../features/notes/editor/noteEditorSchema";
import { fixtureAssets } from "../../../app/previews/hbs-adapters/fixtureAssets";
import { noteFixture } from "../../../app/previews/hbs-adapters/fixtures";
import { changeNoteExample } from "../../../app/previews/hbs-adapters/noteExternalChanges";
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
function paragraph(id: string, text = id.toUpperCase(), children: readonly NoteBlockV1[] = []): NoteBlockV1 {
  return {
    id: noteBlockId(id), type: "paragraph", props: { textColor: "default", backgroundColor: "default", textAlignment: "left" },
    content: { kind: "inline", items: [{ type: "text", text, styles: [] }] }, children,
  };
}
function sample(blocks = [paragraph("a"), paragraph("b"), paragraph("c"), paragraph("d")]): NoteDocumentV1 {
  return { ...noteFixture, blocks };
}
const textPath = (id: string) => ["blocks", { key: "id", value: id }, "content", "items", 0, "text"] as const;
const memberIds = (element: HTMLElement): string[] => JSON.parse(element.dataset.hbsReviewDecisions ?? "[]") as string[];

async function setup(initial = sample()) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const adapter = createNoteDocumentAdapter(directTextDiff);
  let saved: DocumentSnapshot<NoteDocumentV1> = { version: 1, saved: initial, baseline: initial };
  let editor: NoteEditorInstance | undefined;
  const session = await HbsSession.open({
    adapter, requestId: sequence => `review-group-${sequence}`, preservation: { load: () => undefined, save: () => {} },
    port: {
      read: async () => saved,
      commit: async request => {
        expect(request.expectedVersion).toBe(saved.version);
        saved = { version: saved.version + 1, saved: request.saved, baseline: request.baseline };
        return saved;
      },
    },
  });
  const host = document.createElement("div"), controls = document.createElement("div"), surface = document.createElement("div");
  document.body.append(host); host.append(controls, surface);
  const root = createRoot(surface);
  cleanups.push(async () => { await act(async () => root.unmount()); session.dispose(); host.remove(); });
  await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
    <HbsNoteEditor session={session} diffControlsTarget={controls} onEditorReady={value => { editor = value; }}/>
  </NotesProvider>));
  await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
  return {
    adapter, session, host, controls,
    get saved() { return saved; },
    get editor() { if (!editor) throw new Error("editor not mounted"); return editor; },
    groups: () => [...host.querySelectorAll<HTMLElement>("[data-hbs-review-decisions]")],
    external: async (document: NoteDocumentV1) => {
      saved = { ...saved, version: saved.version + 1, saved: document };
      await act(async () => session.syncLatest());
    },
    click: async (group: HTMLElement, action: "Keep" | "Undo" | "Take") => {
      const button = [...group.querySelectorAll("button")].find(button => button.textContent === action);
      if (!button) throw new Error(`missing group action ${action}`);
      await act(async () => button.click());
    },
  };
}

it("reviews adjacent changed blocks together while preserving original decisions and one native history step", async () => {
  const initial = sample(), f = await setup(initial), native = f.editor.prosemirrorView;
  const incoming = f.adapter.replace(f.adapter.replace(initial, textPath("a"), "Changed A"), textPath("b"), "Changed B");
  await f.external(incoming);
  const changes = f.session.getSnapshot().changes;
  expect(changes).toHaveLength(2);
  expect(f.groups()).toHaveLength(1);
  expect(memberIds(f.groups()[0]!)).toEqual(expect.arrayContaining(changes.map(change => change.id)));
  expect(memberIds(f.groups()[0]!)).toHaveLength(2);
  expect(f.controls.querySelector('[aria-label="当前变更位置"]')?.textContent).toBe("1/1");
  expect(f.host.querySelectorAll(".note-diff-added-block")).toHaveLength(2);
  expect(f.host.querySelectorAll(".note-diff-deleted-blocks")).toHaveLength(1);
  expect(f.host.querySelectorAll(".note-diff-deleted-blocks > .bn-block-group > *")).toHaveLength(2);
  await act(async () => f.editor.transact(transaction => { closeHistory(transaction); }));
  const depth = undoDepth(f.editor.prosemirrorState);

  await f.click(f.groups()[0]!, "Keep");

  expect(f.saved.baseline).toEqual(incoming);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.groups()).toHaveLength(0);
  expect(undoDepth(f.editor.prosemirrorState)).toBe(depth + 1);
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(f.saved.baseline).toEqual(initial);
  expect(f.session.getSnapshot().content).toEqual(incoming);
  expect(f.session.getSnapshot().changes).toHaveLength(2);
  expect(f.groups()).toHaveLength(1);
  await act(async () => { expect(f.editor.redo()).toBe(true); });
  expect(f.saved.baseline).toEqual(incoming);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.editor.prosemirrorView).toBe(native);
});

it.each(["stable content", ""])("keeps an unchanged sibling (%j) as a group boundary and reviews only the selected run", async gap => {
  const initial = sample([paragraph("a"), paragraph("b"), paragraph("gap", gap), paragraph("d")]);
  const f = await setup(initial);
  const incoming = ["a", "b", "d"].reduce((document, id) => f.adapter.replace(document, textPath(id), `Changed ${id}`), initial);
  await f.external(incoming);
  const decisions = f.session.getSnapshot().changes;
  expect(decisions).toHaveLength(3);
  expect(f.groups()).toHaveLength(2);
  expect(f.groups().map(group => memberIds(group).length)).toEqual([2, 1]);
  expect(f.controls.querySelector('[aria-label="当前变更位置"]')?.textContent).toBe("1/2");

  await f.click(f.groups()[0]!, "Keep");

  expect(f.adapter.read(f.saved.baseline, textPath("a"))).toBe("Changed a");
  expect(f.adapter.read(f.saved.baseline, textPath("b"))).toBe("Changed b");
  expect(f.adapter.read(f.saved.baseline, textPath("d"))).toBe("D");
  expect(f.adapter.read(f.saved.saved, textPath("d"))).toBe("Changed d");
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.groups()).toHaveLength(1);
  expect(memberIds(f.groups()[0]!)).toEqual([decisions.find(change => change.path.some(part => typeof part === "object" && part.value === "d"))!.id]);
});

it("keeps consecutive deletion snapshots in source order with one Undo that restores the run", async () => {
  const initial = sample(), f = await setup(initial);
  await f.external({ ...initial, blocks: [initial.blocks[0]!, initial.blocks[3]!] });
  expect(f.session.getSnapshot().changes).toHaveLength(2);
  expect([...f.host.querySelectorAll(".note-diff-deleted-blocks > .bn-block-group > *")].map(element => element.textContent)).toEqual(["B", "C"]);
  expect(f.groups()).toHaveLength(1);
  expect(memberIds(f.groups()[0]!)).toHaveLength(2);

  await f.click(f.groups()[0]!, "Undo");

  expect(f.session.getSnapshot().content).toEqual(initial);
  expect(f.saved.saved).toEqual(initial);
  expect(f.saved.baseline).toEqual(initial);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(f.host.querySelectorAll(".note-diff-deleted-blocks")).toHaveLength(0);
});

it("groups nested siblings within their parent and keeps the adjacent root decision separate", async () => {
  const initial = sample([paragraph("a"), paragraph("parent", "Parent", [paragraph("child-a"), paragraph("child-b")]), paragraph("tail")]);
  const f = await setup(initial);
  const incoming: NoteDocumentV1 = {
    ...initial,
    blocks: [paragraph("a", "Changed root"), paragraph("parent", "Parent", [paragraph("child-a", "Changed first child"), paragraph("child-b", "Changed second child")]), paragraph("tail")],
  };
  await f.external(incoming);
  expect(f.session.getSnapshot().changes).toHaveLength(3);
  expect(f.groups()).toHaveLength(2);
  const nested = f.groups().find(group => memberIds(group).length === 2)!;
  expect(nested.closest('[data-node-type="blockContainer"][data-id="parent"]')).not.toBeNull();
  expect(f.groups().find(group => memberIds(group).length === 1)?.closest('[data-id="parent"]')).toBeNull();

  await f.click(nested, "Keep");

  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.adapter.read(f.saved.baseline, textPath("a"))).toBe("A");
  expect(f.saved.baseline.blocks[1]!.children).toEqual(incoming.blocks[1]!.children);
  expect(f.groups()).toHaveLength(1);
});

it("groups the preview's adjacent code, divider, image, attachment and checklist candidates across updates", async () => {
  const f = await setup(noteFixture);
  let incoming = noteFixture;
  const kinds = ["codeBlock", "divider", "image", "file", "checkListItem"];
  for (const [index, kind] of kinds.entries()) {
    incoming = changeNoteExample(incoming, kind, index + 2, fixtureAssets);
    await f.external(incoming);
    expect(f.groups()).toHaveLength(1);
    expect(memberIds(f.groups()[0]!)).toHaveLength(index + 1);
  }
  expect(f.session.getSnapshot().changes).toHaveLength(5);
  expect(f.host.querySelectorAll(".note-diff-added-block")).toHaveLength(5);
  expect(incoming.blocks.slice(-5).map(block => block.type)).toEqual(kinds);

  await f.click(f.groups()[0]!, "Undo");

  expect(f.session.getSnapshot().content).toEqual(noteFixture);
  expect(f.saved.saved).toEqual(noteFixture);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});

it("keeps adjacent ordinary candidates outside a conflict group's Take action", async () => {
  const initial = sample([paragraph("a", "Score A 80"), paragraph("b", "Score B 80"), paragraph("tail")]), f = await setup(initial);
  const first = f.adapter.replace(f.adapter.replace(initial, textPath("a"), "Score A 85"), textPath("b"), "Score B 85");
  await f.external(first);
  await act(async () => f.editor.updateBlock("b", { content: "Score B 86" }));
  await f.external(f.adapter.replace(first, textPath("b"), "Score B 87"));
  expect(f.session.getSnapshot().conflicts.length).toBeGreaterThan(0);
  expect(f.groups()).toHaveLength(2);
  const conflict = f.groups().find(group => [...group.querySelectorAll("button")].some(button => button.textContent === "Take"))!;
  expect(memberIds(conflict)).toHaveLength(1);
  expect(f.host.querySelector('[aria-label="incoming"]')?.textContent).toBe("Score B 87");

  await f.click(conflict, "Take");

  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(f.adapter.read(f.session.getSnapshot().content, textPath("a"))).toBe("Score A 85");
  expect(f.adapter.read(f.saved.baseline, textPath("a"))).toBe("Score A 80");
  expect(f.adapter.read(f.session.getSnapshot().content, textPath("b"))).toBe("Score B 87");
  expect(f.session.getSnapshot().changes).toHaveLength(2);
});

it("shares Take across adjacent conflicts without accepting either candidate", async () => {
  const initial = sample([paragraph("a", "Score A 80"), paragraph("b", "Score B 80"), paragraph("tail")]), f = await setup(initial);
  const candidates = ["a", "b"].reduce((document, id) => f.adapter.replace(document, textPath(id), `Score ${id.toUpperCase()} 85`), initial);
  await f.external(candidates);
  await act(async () => {
    f.editor.updateBlock("a", { content: "Score A 86" });
    f.editor.updateBlock("b", { content: "Score B 86" });
  });
  await f.external(["a", "b"].reduce((document, id) => f.adapter.replace(document, textPath(id), `Score ${id.toUpperCase()} 87`), candidates));
  const conflicts = f.session.getSnapshot().conflicts;
  expect(conflicts).toHaveLength(2);
  expect(f.groups()).toHaveLength(1);
  expect(memberIds(f.groups()[0]!)).toEqual(expect.arrayContaining(conflicts.map(conflict => conflict.id)));
  expect(f.host.querySelectorAll('[aria-label="incoming"]')).toHaveLength(1);
  expect(f.host.querySelectorAll('[aria-label="incoming"] > .bn-block-group > *')).toHaveLength(2);

  await f.click(f.groups()[0]!, "Take");

  expect(f.session.getSnapshot().conflicts).toHaveLength(0);
  expect(f.adapter.read(f.session.getSnapshot().content, textPath("a"))).toBe("Score A 87");
  expect(f.adapter.read(f.session.getSnapshot().content, textPath("b"))).toBe("Score B 87");
  expect(f.saved.baseline).toEqual(initial);
  expect(f.session.getSnapshot().changes).toHaveLength(2);
  expect(f.groups()).toHaveLength(1);
});

it("keeps a beginning insertion separate from an end deletion across unchanged siblings on both sides", async () => {
  const initial = sample([paragraph("a"), paragraph("b"), paragraph("c")]), f = await setup(initial);
  await f.external({ ...initial, blocks: [paragraph("x"), initial.blocks[0]!, initial.blocks[1]!] });
  const changes = f.session.getSnapshot().changes;
  expect(changes).toHaveLength(2);
  expect(f.groups()).toHaveLength(2);
  expect(f.groups().map(group => memberIds(group).length)).toEqual([1, 1]);
  expect(f.host.querySelector(".note-diff-deleted-blocks")?.textContent).toBe("C");

  await f.click(f.groups()[0]!, "Undo");

  expect(f.saved.saved.blocks.map(block => block.id)).toEqual(["a", "b"]);
  expect(f.saved.baseline).toEqual(initial);
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.session.getSnapshot().changes[0]?.after).toBeUndefined();
  expect(f.groups()).toHaveLength(1);
  await f.click(f.groups()[0]!, "Keep");
  expect(f.saved.baseline.blocks.map(block => block.id)).toEqual(["a", "b"]);
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});

it("groups insertion and deletion at the same sibling gap without consuming a separated candidate", async () => {
  const initial = sample(), f = await setup(initial);
  const incoming = { ...initial, blocks: [initial.blocks[0]!, paragraph("replacement", "Replacement B"), initial.blocks[2]!, paragraph("d", "Changed D")] };
  await f.external(incoming);
  expect(f.session.getSnapshot().changes).toHaveLength(3);
  expect(f.groups()).toHaveLength(2);
  expect(f.groups().map(group => memberIds(group).length)).toEqual([2, 1]);
  expect(f.host.querySelector(".note-diff-deleted-blocks")?.textContent).toBe("B");

  await f.click(f.groups()[0]!, "Undo");

  expect(f.saved.saved.blocks.map(block => block.id)).toEqual(["a", "b", "c", "d"]);
  expect(f.adapter.read(f.saved.saved, textPath("b"))).toBe("B");
  expect(f.adapter.read(f.saved.saved, textPath("d"))).toBe("Changed D");
  expect(f.saved.baseline).toEqual(initial);
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.groups()).toHaveLength(1);
});
