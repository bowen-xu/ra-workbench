// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { undoDepth, redoDepth } from "@tiptap/pm/history";
import { afterEach, expect, it, vi } from "vitest";
import { HbsSession, createNoteDocumentAdapter, projectDocumentReview, type DocumentReviewProjector, type DocumentSnapshot } from "@research-assistant/core/hbs";
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
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => { for (const cleanup of cleanups.splice(0)) await cleanup(); vi.unstubAllGlobals(); });
const ids = (document: NoteDocumentV1) => document.blocks.map(block => block.id);
function moved(document: NoteDocumentV1): NoteDocumentV1 {
  const blocks = [...document.blocks], last = blocks.pop();
  if (!last) throw new Error("empty fixture");
  blocks.splice(1, 0, last); return { ...document, blocks };
}
async function setup(initial = noteFixture, reviewProjector?: DocumentReviewProjector) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const adapter = createNoteDocumentAdapter(directTextDiff);
  let saved: DocumentSnapshot<NoteDocumentV1> = { version: 1, saved: initial, baseline: initial }, editor: NoteEditorInstance | undefined;
  const session = await HbsSession.open({ adapter, requestId: n => `projection-${n}`, preservation: { load: () => undefined, save: () => {} },
    port: { read: async () => saved, commit: async request => {
      expect(request.expectedVersion).toBe(saved.version);
      saved = { version: saved.version + 1, saved: request.saved, baseline: request.baseline }; return saved;
    } } });
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  cleanups.push(async () => { await act(async () => root.unmount()); session.dispose(); host.remove(); });
  const render = async (projector = reviewProjector) => {
    await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}>
      <HbsNoteEditor session={session} {...(projector ? { reviewProjector: projector } : {})} onEditorReady={value => { editor = value; }}/>
    </NotesProvider>));
    await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
  };
  await render();
  return { adapter, session, host, render, get saved() { return saved; },
    get editor() { if (!editor) throw new Error("editor not mounted"); return editor; },
    external: async (document: NoteDocumentV1) => {
      saved = { ...saved, version: saved.version + 1, saved: document }; await act(async () => session.syncLatest());
    },
    click: async (label: string) => {
      const button = [...host.querySelectorAll("button")].find(button => button.getAttribute("aria-label") === label || button.textContent === label);
      if (!button) throw new Error(`missing ${label}`); await act(async () => button.click());
    },
  };
}

it("displays the real eight-block move only at its old and current locations", async () => {
  const f = await setup(), native = f.editor.prosemirrorView;
  await f.external(moved(noteFixture));
  const decision = f.session.getSnapshot().changes[0];
  expect(decision?.kind).toBe("order");
  expect(f.host.querySelectorAll(".note-diff-deleted-blocks")).toHaveLength(1);
  expect(f.host.querySelector('[aria-label="原位置"] .bn-block-group')?.textContent).toBe("下一步：补充复杂场景的验证。");
  const current = f.host.querySelectorAll(".note-diff-added-block"); expect(current).toHaveLength(1);
  expect(current[0]?.getAttribute("data-id")).toBe("hbs-next");
  expect(f.host.querySelectorAll(".note-diff-added-text,.note-diff-deleted-text,.note-diff-added-atom,.note-diff-deleted-atom")).toHaveLength(0);
  expect(f.host.querySelectorAll("[data-hbs-review-decisions]")).toHaveLength(2);
  expect(f.editor.prosemirrorView).toBe(native);
  expect(ids(f.session.getSnapshot().content)).toEqual(ids(moved(noteFixture)));
  expect(new Set(ids(f.session.getSnapshot().content)).size).toBe(8);
  expect(ids(f.saved.baseline)).toEqual(ids(noteFixture));
});

it("places a nested sequence's old fragment inside its own parent without marking parent text", async () => {
  const f = await setup();
  const incoming = { ...noteFixture, blocks: noteFixture.blocks.map(block => block.id === "hbs-method" ? { ...block, children: [...block.children].reverse() } : block) };
  await f.external(incoming);
  const parent = f.host.querySelector('[data-node-type="blockContainer"][data-id="hbs-method"]');
  const old = f.host.querySelector('[aria-label="原位置"]');
  expect(old?.closest('[data-node-type="blockContainer"][data-id="hbs-method"]')).toBe(parent);
  expect(old?.querySelector(".bn-block-group")?.textContent).toBe("记录每次实验的误差。");
  expect(parent?.classList.contains("note-diff-added-block")).toBe(false);
  expect(f.host.querySelectorAll(".note-diff-added-block")).toHaveLength(1);
  expect(f.host.querySelector(".note-diff-added-block")?.getAttribute("data-id")).toBe("hbs-nested-b");
  expect(f.host.querySelectorAll("[data-hbs-review-decisions]")).toHaveLength(2);
});

it("shares one original order decision across all fourteen location endpoints and native review history", async () => {
  const f = await setup(); await f.external({ ...noteFixture, blocks: [...noteFixture.blocks].reverse() });
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.host.querySelectorAll('[data-hbs-review-placement="before"]')).toHaveLength(7);
  expect(f.host.querySelectorAll('[data-hbs-review-placement="after"]')).toHaveLength(7);
  expect(f.host.querySelectorAll(".note-diff-added-text,.note-diff-deleted-text,.note-diff-added-atom,.note-diff-deleted-atom,.note-diff-added-block-atom,.note-diff-deleted-block-atom")).toHaveLength(0);
  const endpoints = [...f.host.querySelectorAll<HTMLElement>("[data-hbs-review-decisions]")];
  const decision = f.session.getSnapshot().changes[0];
  expect(endpoints).toHaveLength(14);
  expect(endpoints.every(endpoint => endpoint.dataset.hbsReviewDecisions === JSON.stringify([decision?.id]))).toBe(true);
  expect(endpoints.filter(endpoint => endpoint.dataset.hbsReviewSide === "before")).toHaveLength(7);
  expect(endpoints.filter(endpoint => endpoint.dataset.hbsReviewSide === "after")).toHaveLength(7);
  await f.click("Keep");
  expect(f.session.getSnapshot().changes).toHaveLength(0);
  expect(ids(f.saved.baseline)).toEqual([...ids(noteFixture)].reverse());
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.host.querySelectorAll("[data-hbs-review-decisions]")).toHaveLength(14);
  await act(async () => { expect(f.editor.redo()).toBe(true); });
  expect(f.session.getSnapshot().changes).toHaveLength(0);
});

it.each(["keep", "undo"] as const)("keeps a moved member's content decision separate when resolving its order with %s", async action => {
  const f = await setup();
  const path = ["blocks", { key: "id", value: "hbs-next" }, "content", "items", 0, "text"] as const;
  await f.external(moved(f.adapter.replace(noteFixture, path, "独立候选正文。")));
  expect(f.session.getSnapshot().changes.map(change => change.kind).sort()).toEqual(["order", "text"]);
  expect(f.host.querySelectorAll("[data-hbs-review-decisions]")).toHaveLength(3);
  const order = f.session.getSnapshot().changes.find(change => change.kind === "order");
  if (!order) throw new Error("missing order decision");
  const actions = [...f.host.querySelectorAll<HTMLElement>("[data-hbs-review-decisions]")]
    .find(host => host.dataset.hbsReviewSide === (action === "keep" ? "after" : "before")
      && (JSON.parse(host.dataset.hbsReviewDecisions ?? "[]") as string[]).includes(order.id));
  const button = [...(actions?.querySelectorAll("button") ?? [])].find(button => button.textContent === (action === "keep" ? "Keep" : "Undo"));
  if (!button) throw new Error("missing order action");
  await act(async () => button.click());
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.session.getSnapshot().changes[0]?.kind).toBe("text");
  expect(f.adapter.read(f.session.getSnapshot().content, path)).toBe("独立候选正文。");
  expect(f.adapter.read(f.saved.baseline, path)).toBe("下一步：补充复杂场景的验证。");
  expect(ids(f.session.getSnapshot().content)).toEqual(action === "keep" ? ids(moved(noteFixture)) : ids(noteFixture));
  await act(async () => { expect(f.editor.undo()).toBe(true); });
  expect(f.session.getSnapshot().changes).toHaveLength(2);
  await act(async () => { expect(f.editor.redo()).toBe(true); });
  expect(f.session.getSnapshot().changes).toHaveLength(1);
});

it("anchors a deleted nested member before its surviving sibling instead of the document end", async () => {
  const f = await setup();
  await f.external({ ...noteFixture, blocks: noteFixture.blocks.map(block => block.id === "hbs-method" ? { ...block, children: block.children.slice(1) } : block) });
  const old = f.host.querySelector(".note-diff-deleted-blocks"), sibling = f.host.querySelector('[data-id="hbs-nested-b"]');
  expect(old?.textContent).toBe("重复实验 3 次。");
  expect(old?.closest('[data-id="hbs-method"]')).not.toBeNull();
  expect(sibling).not.toBeNull();
  expect(old!.compareDocumentPosition(sibling!) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(f.host.querySelectorAll(".note-diff-added-block")).toHaveLength(0);
});

it("replaces the pure projection strategy without replacing native content or accepting its decision", async () => {
  const first = vi.fn(projectDocumentReview), f = await setup(noteFixture, first);
  await f.external(moved(noteFixture));
  const native = f.editor.prosemirrorView, content = f.session.getSnapshot().content;
  const replacement: DocumentReviewProjector = change => projectDocumentReview(change);
  await f.render(replacement);
  expect(first).toHaveBeenCalled();
  expect(f.editor.prosemirrorView).toBe(native);
  expect(f.session.getSnapshot().content).toEqual(content);
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.host.querySelectorAll(".note-diff-added-block")).toHaveLength(1);
});

it("keeps an independent collection decision visible beside a member's content conflict", async () => {
  const f = await setup();
  const path = ["blocks", { key: "id", value: "hbs-next" }, "content", "items", 0, "text"] as const;
  await f.external(f.adapter.replace(noteFixture, path, "第一版候选。"));
  await act(async () => f.editor.updateBlock("hbs-next", { content: "我的候选编辑。" }));
  await f.external(moved(f.adapter.replace(f.saved.saved, path, "第二版候选。")));
  const state = f.session.getSnapshot(), order = state.changes.find(change => change.kind === "order");
  expect(state.conflicts.length).toBeGreaterThan(0);
  expect(order).toBeDefined();
  if (!order) throw new Error("missing independent order");
  const controls = [...f.host.querySelectorAll<HTMLElement>("[data-hbs-review-decisions]")];
  expect(controls.some(host => (JSON.parse(host.dataset.hbsReviewDecisions ?? "[]") as string[]).includes(order.id))).toBe(true);
  expect(f.host.querySelector('[aria-label="incoming"]')?.textContent).toContain("第二版候选。");
  expect(f.adapter.read(state.content, path)).toBe("我的候选编辑。");
});

it("keeps consecutive deletions at an emptied nested collection's own boundary", async () => {
  const f = await setup();
  await f.external({ ...noteFixture, blocks: noteFixture.blocks.map(block => block.id === "hbs-method" ? { ...block, children: [] } : block) });
  const parent = f.host.querySelector('[data-node-type="blockContainer"][data-id="hbs-method"]');
  const old = [...f.host.querySelectorAll(".note-diff-deleted-blocks")];
  expect(old).toHaveLength(1);
  expect([...old[0]!.querySelectorAll(":scope > .bn-block-group > *")].map(fragment => fragment.textContent))
    .toEqual(["重复实验 3 次。", "记录每次实验的误差。"]);
  expect(old.every(fragment => fragment.closest('[data-node-type="blockContainer"][data-id="hbs-method"]') === parent)).toBe(true);
  expect(parent?.classList.contains("note-diff-added-block")).toBe(false);
  expect(f.session.getSnapshot().content.blocks.find(block => block.id === "hbs-method")?.children).toEqual([]);
});

it("jumps between old and new controls without resolving, rewriting input, or adding native history", async () => {
  const f = await setup(); await f.external(moved(noteFixture));
  const before = f.host.querySelector<HTMLElement>('[data-hbs-review-location][data-hbs-review-side="before"]');
  const after = f.host.querySelector<HTMLElement>('[data-hbs-review-location][data-hbs-review-side="after"]');
  if (!before || !after) throw new Error("missing placement endpoints");
  const oldJump = after.querySelector<HTMLButtonElement>("[data-review-location-jump]");
  const newJump = before.querySelector<HTMLButtonElement>("[data-review-location-jump]");
  if (!oldJump || !newJump) throw new Error("missing navigation buttons");
  expect(oldJump.textContent).toBe("old"); expect(newJump.textContent).toBe("new");
  expect([...after.querySelectorAll("button")].map(button => button.textContent)).toEqual(["old", "Keep"]);
  expect([...before.querySelectorAll("button")].map(button => button.textContent)).toEqual(["new", "Undo"]);
  expect(before.dataset.hbsReviewLocation).toBe(after.dataset.hbsReviewLocation);
  const scrollOld = vi.spyOn(before, "scrollIntoView"), scrollNew = vi.spyOn(after, "scrollIntoView");
  const state = f.session.getSnapshot(), saved = f.saved, native = f.editor.prosemirrorView;
  const undoBefore = undoDepth(f.editor.prosemirrorState), redoBefore = redoDepth(f.editor.prosemirrorState);
  await act(async () => oldJump.click());
  expect(scrollOld).toHaveBeenCalledExactlyOnceWith({ block: "nearest" });
  expect(document.activeElement).toBe(newJump);
  await act(async () => newJump.click());
  expect(scrollNew).toHaveBeenCalledExactlyOnceWith({ block: "nearest" });
  expect(document.activeElement).toBe(oldJump);
  expect(f.session.getSnapshot().content).toBe(state.content);
  expect(f.session.getSnapshot().baselineProjection).toBe(state.baselineProjection);
  expect(f.session.getSnapshot().changes).toEqual(state.changes);
  expect(f.saved).toBe(saved); expect(f.editor.prosemirrorView).toBe(native);
  expect(undoDepth(f.editor.prosemirrorState)).toBe(undoBefore);
  expect(redoDepth(f.editor.prosemirrorState)).toBe(redoBefore);
});

it("scopes location navigation to its own native editor when two instances share decision and block identities", async () => {
  const first = await setup(), second = await setup();
  await first.external(moved(noteFixture)); await second.external(moved(noteFixture));
  const firstBefore = first.host.querySelector<HTMLElement>('[data-hbs-review-location][data-hbs-review-side="before"]');
  const secondBefore = second.host.querySelector<HTMLElement>('[data-hbs-review-location][data-hbs-review-side="before"]');
  const firstAfter = first.host.querySelector<HTMLElement>('[data-hbs-review-location][data-hbs-review-side="after"]');
  if (!firstBefore || !secondBefore || !firstAfter) throw new Error("missing two-instance endpoints");
  expect(firstBefore.dataset.hbsReviewLocation).toBe(secondBefore.dataset.hbsReviewLocation);
  const scrollOwn = vi.spyOn(firstBefore, "scrollIntoView"), scrollOther = vi.spyOn(secondBefore, "scrollIntoView");
  const jump = firstAfter.querySelector<HTMLButtonElement>("[data-review-location-jump]");
  if (!jump) throw new Error("missing first instance old jump");
  const secondState = second.session.getSnapshot();
  await act(async () => jump.click());
  expect(scrollOwn).toHaveBeenCalledOnce(); expect(scrollOther).not.toHaveBeenCalled();
  expect(document.activeElement).toBe(firstBefore.querySelector("[data-review-location-jump]"));
  expect(second.session.getSnapshot()).toBe(secondState);
});

it("keeps deleted and relocated old fragments in their original order before the same stable successor", async () => {
  const sample = noteFixture.blocks.find(block => block.id === "hbs-score");
  if (!sample) throw new Error("missing paragraph fixture");
  const initial: NoteDocumentV1 = { ...noteFixture, blocks: ["a", "b", "c", "d"].map(id => ({
    ...sample, id: noteBlockId(id), children: [], content: { kind: "inline", items: [{ type: "text", text: id.toUpperCase(), styles: [] }] },
  })) };
  const f = await setup(initial), incoming: NoteDocumentV1 = { ...initial, blocks: [initial.blocks[2]!, initial.blocks[0]!, initial.blocks[3]!] };
  await f.external(incoming);
  const state = f.session.getSnapshot(), order = state.changes.find(change => change.kind === "order"), deletion = state.changes.find(change => change.after === undefined);
  if (!order || !deletion) throw new Error("missing independent order/deletion decisions");
  expect(state.changes).toHaveLength(2); expect(order.id).not.toBe(deletion.id);
  const old = [...f.host.querySelectorAll<HTMLElement>(".note-diff-deleted-blocks")];
  const oldB = old.find(fragment => fragment.querySelector(".bn-block-group")?.textContent === "B");
  const oldC = old.find(fragment => fragment.querySelector(".bn-block-group")?.textContent === "C");
  const currentA = f.host.querySelector('[data-node-type="blockContainer"][data-id="a"]');
  const currentD = f.host.querySelector('[data-node-type="blockContainer"][data-id="d"]');
  if (!oldB || !oldC || !currentA || !currentD) throw new Error("missing current/old sequence members");
  // Both old members belong to the old gap before stable d; moved c cannot
  // become b's successor simply because it still exists at a new location.
  expect(currentA.compareDocumentPosition(oldB) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(oldB.compareDocumentPosition(oldC) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  expect(oldC.compareDocumentPosition(currentD) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  const controls = [...f.host.querySelectorAll<HTMLElement>("[data-hbs-review-decisions]")];
  expect(controls).toHaveLength(3);
  expect(controls.filter(host => host.dataset.hbsReviewDecisions === JSON.stringify([order.id]))).toHaveLength(2);
  expect(controls.filter(host => host.dataset.hbsReviewDecisions === JSON.stringify([deletion.id]))).toHaveLength(1);
  const orderKeep = controls.find(host => host.dataset.hbsReviewDecisions === JSON.stringify([order.id]) && host.dataset.hbsReviewSide === "after")?.querySelector<HTMLButtonElement>(".writing-diff-chunk-keep");
  if (!orderKeep) throw new Error("missing order keep endpoint");
  await act(async () => orderKeep.click());
  expect(ids(f.session.getSnapshot().content)).toEqual(["c", "a", "d"]);
  expect(ids(f.saved.baseline)).toContain("b");
  expect(f.session.getSnapshot().changes).toHaveLength(1);
  expect(f.session.getSnapshot().changes[0]?.after).toBeUndefined();
});
