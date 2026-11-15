// @vitest-environment happy-dom
import { BlockNoteEditor, createExtension } from "@blocknote/core";
import { closeHistory } from "@tiptap/pm/history";
import { Step, StepResult } from "@tiptap/pm/transform";
import type { Node } from "@tiptap/pm/model";
import { afterEach, describe, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { appendNoteReviewBaseline, createNoteReviewHistory, readNoteReviewHistory, NOTE_REVIEW_ORIGIN, type NoteReviewBaselineSynchronizer } from "./noteReviewHistory";

const pageId = notePageId("review-page");
const stops: (() => void)[] = [];
afterEach(() => { stops.splice(0).forEach(stop => stop()); });
const document = (first = "Old one", second = "Old two"): NoteDocumentV1 => ({ schemaVersion: 1, pageId, blocks: [first, second].map((text, index) => ({
  id: noteBlockId(index ? "two" : "one"), type: "paragraph", props: { futureMetadata: "kept" }, children: [],
  content: { kind: "inline", items: [{ type: "text", text, styles: ["bold"] }] },
})) });

function setup(baseline = document(), syncBaseline?: NoteReviewBaselineSynchronizer) {
  const changes = vi.fn();
  const editor = BlockNoteEditor.create({
    extensions: [createExtension(() => ({ key: "test-review", prosemirrorPlugins: [createNoteReviewHistory(baseline, changes, syncBaseline)] }))()],
    initialContent: [{ id: "one", type: "paragraph", content: "Old one" }, { id: "two", type: "paragraph", content: "Old two" }],
  });
  const host = globalThis.document.createElement("div"); globalThis.document.body.append(host); editor.mount(host);
  stops.push(() => { editor.unmount(); host.remove(); });
  const state = () => editor.prosemirrorState;
  const review = () => readNoteReviewHistory(state())!;
  const baselineChange = (next: NoteDocumentV1, requestId: string, update?: () => void) => {
    const previous = state();
    editor.transact(tr => { update?.(); appendNoteReviewBaseline(previous, tr, next, { origin: "review", requestId }); });
    editor.transact(tr => { closeHistory(tr); });
  };
  return { editor, host, state, review, changes, baselineChange };
}

describe("Note review baseline in the original ProseMirror history", () => {
  it("retains installation and replay proof through document-neutral history bookkeeping", () => {
    class BookkeepingStep extends Step {
      apply(doc: Node) { return StepResult.ok(doc); }
      invert() { return this; }
      map() { return this; }
      toJSON() { return { stepType: "test-bookkeeping" }; }
    }
    const { editor, state, review, baselineChange } = setup();
    editor.prosemirrorView.dispatch(appendNoteReviewBaseline(state(), state().tr, document(), { origin: "agent" }));
    const installation = review();
    const bookkeep = () => editor.prosemirrorView.dispatch(state().tr.step(new BookkeepingStep()).setMeta("addToHistory", false));
    bookkeep();
    expect(review()).toBe(installation);
    expect(review().installationId).toBeTypeOf("string");
    baselineChange(document("Agent one"), "keep-proof");
    editor.undo();
    const replay = review();
    bookkeep();
    expect(review()).toBe(replay);
    expect(review()).toMatchObject({ historyProofRequestId: "keep-proof", direction: "undo" });
    editor.updateBlock("one", { content: "Manual after undo" });
    expect(review().historyProofRequestId).toBeUndefined();
  });
  it("retains a distinct local installation identity through Agent undo/redo and Step serialization", () => {
    const { editor, state, review } = setup();
    const tr = appendNoteReviewBaseline(state(), state().tr, document(), { origin: "agent" });
    const encoded = tr.steps[0]!.toJSON();
    expect(Step.fromJSON(state().schema, encoded).toJSON()).toEqual(encoded);
    editor.prosemirrorView.dispatch(tr);
    const installationId = review().installationId;
    expect(installationId).toBeTypeOf("string"); expect(review().historyProofRequestId).toBeUndefined();
    editor.undo(); expect(review()).toMatchObject({ installationId, direction: "undo", origin: "agent" });
    editor.redo(); expect(review()).toMatchObject({ installationId, direction: "redo", origin: "agent" });
    expect(() => Step.fromJSON(state().schema, { ...encoded, installationId: "" })).toThrow();
    expect(() => Step.fromJSON(state().schema, { ...encoded, origin: "manual" })).toThrow();
  });
  it("suppresses old-caret scrolling only for review-decision history, not navigation or ordinary input", () => {
    const { editor, baselineChange } = setup();
    baselineChange(document("Kept"), "keep-request");
    const handled = () => editor.prosemirrorView.someProp("handleScrollToSelection", handler => handler(editor.prosemirrorView));
    expect(editor.undo()).toBe(true); expect(handled()).toBe(true);
    expect(editor.redo()).toBe(true); expect(handled()).toBe(true);
    editor.setTextCursorPosition("two", "end"); expect(handled()).not.toBe(true);
    editor.updateBlock("two", { content: "Manual input" });
    expect(editor.undo()).toBe(true); expect(handled()).not.toBe(true);
  });
  it("undoes and redoes a baseline-only Keep after B equals D, retaining editor identity and proof", () => {
    const { editor, host, state, review, changes, baselineChange } = setup();
    const previous = state();
    editor.transact(tr => {
      editor.updateBlock("one", { content: "Agent one" });
      appendNoteReviewBaseline(previous, tr, document(), { origin: "agent" });
    });
    editor.transact(tr => { closeHistory(tr); });
    const currentDom = host.querySelector(".ProseMirror");
    changes.mockClear();
    baselineChange(document("Agent one"), "keep-request");
    expect(review().baseline).toEqual(document("Agent one"));
    expect(changes).toHaveBeenCalledOnce();
    expect(editor.undo()).toBe(true);
    expect(editor.getBlock("one")?.content).toEqual([{ type: "text", text: "Agent one", styles: {} }]);
    expect(review()).toMatchObject({ baseline: document(), direction: "undo", historyProofRequestId: "keep-request" });
    expect(editor.redo()).toBe(true);
    expect(review()).toMatchObject({ baseline: document("Agent one"), direction: "redo", historyProofRequestId: "keep-request" });
    expect(host.querySelector(".ProseMirror")).toBe(currentDom);
    expect(changes).toHaveBeenCalledTimes(3);
  });

  it("keeps successive individual decisions as separate history items", () => {
    const { editor, review, baselineChange } = setup();
    baselineChange(document("Agent one"), "first-keep");
    baselineChange(document("Agent one", "Agent two"), "second-keep");
    expect(editor.undo()).toBe(true);
    expect(review()).toMatchObject({ baseline: document("Agent one"), historyProofRequestId: "second-keep", direction: "undo" });
    expect(editor.undo()).toBe(true);
    expect(review()).toMatchObject({ baseline: document(), historyProofRequestId: "first-keep", direction: "undo" });
    expect(editor.redo()).toBe(true);
    expect(review().baseline).toEqual(document("Agent one"));
    expect(editor.redo()).toBe(true);
    expect(review().baseline).toEqual(document("Agent one", "Agent two"));
  });

  it("places bulk document and baseline review in one reversible history transaction", () => {
    const { editor, review, baselineChange } = setup();
    baselineChange(document("Kept one", "Kept two"), "bulk-request", () => {
      editor.updateBlock("one", { content: "Kept one" }); editor.updateBlock("two", { content: "Kept two" });
    });
    expect(editor.undo()).toBe(true);
    expect(editor.getBlock("one")?.content).toEqual([{ type: "text", text: "Old one", styles: {} }]);
    expect(editor.getBlock("two")?.content).toEqual([{ type: "text", text: "Old two", styles: {} }]);
    expect(review().baseline).toEqual(document());
    expect(editor.undo()).toBe(false);
    expect(editor.redo()).toBe(true);
    expect(review().baseline).toEqual(document("Kept one", "Kept two"));
  });

  it("synchronizes an ordinary edit and B in one input transaction without losing block metadata", () => {
    const { editor, state, review } = setup(document("Old one", "😀 ordinary"));
    const previous = state();
    editor.transact(tr => {
      editor.updateBlock("two", { content: "Ordinary 中文😀" });
      appendNoteReviewBaseline(previous, tr, document("Old one", "Ordinary 中文😀"), { origin: "manual" });
    });
    expect(review().origin).toBe("manual");
    expect(editor.undo()).toBe(true);
    expect(review().baseline).toEqual(document("Old one", "😀 ordinary"));
    expect(review().baseline.blocks[0]?.props).toEqual({ futureMetadata: "kept" });
    expect(review().baseline.blocks[0]?.content).toEqual(document().blocks[0]?.content);
  });

  it("reconciles initial baseline without manufacturing an undo item and exposes operation origin", () => {
    const { editor, state, review } = setup();
    const transaction = appendNoteReviewBaseline(state(), state().tr, document("Restored"), { origin: "restore", addToHistory: false });
    expect(transaction.getMeta(NOTE_REVIEW_ORIGIN)).toBe("restore");
    editor.prosemirrorView.dispatch(transaction);
    expect(review().baseline).toEqual(document("Restored"));
    expect(editor.undo()).toBe(false);
  });

  it("does not reuse a history proof for subsequent D-only manual input", () => {
    const { editor, review, baselineChange } = setup();
    baselineChange(document("Kept"), "keep-proof");
    editor.undo();
    expect(review().historyProofRequestId).toBe("keep-proof");
    editor.updateBlock("one", { content: "Manual after undo" });
    expect(review().baseline).toEqual(document());
    expect(review().origin).toBe("manual");
    expect(review().historyProofRequestId).toBeUndefined();
    expect(review().direction).toBeUndefined();
  });

  it("appends ordinary-region B synchronization to the input's original history event", () => {
    const synchronize = vi.fn<NoteReviewBaselineSynchronizer>((_before, after, baseline) => {
      let ordinaryText = "";
      after.doc.descendants(node => { if (node.attrs.id === "two") ordinaryText = node.textContent; });
      return { ...baseline, blocks: baseline.blocks.map(block => block.id === "two"
        ? { ...block, content: { kind: "inline", items: [{ type: "text", text: ordinaryText, styles: ["bold"] }] } } : block) };
    });
    const { editor, review, state } = setup(document(), synchronize);
    const previous = state();
    editor.transact(tr => {
      editor.updateBlock("one", { content: "Pending Agent one" });
      appendNoteReviewBaseline(previous, tr, document(), { origin: "agent" });
    });
    editor.transact(tr => { closeHistory(tr); });
    expect(synchronize).not.toHaveBeenCalled();
    editor.updateBlock("two", { content: "Ordinary edited" });
    expect(review().baseline).toEqual(document("Old one", "Ordinary edited"));
    expect(editor.undo()).toBe(true);
    expect(review().baseline).toEqual(document());
    expect(editor.getBlock("two")?.content).toEqual([{ type: "text", text: "Old two", styles: {} }]);
    expect(editor.getBlock("one")?.content).toEqual([{ type: "text", text: "Pending Agent one", styles: {} }]);
    expect(synchronize).toHaveBeenCalledOnce(); // Undo replays its recorded inverse; no inference.
    expect(editor.redo()).toBe(true);
    expect(review().baseline).toEqual(document("Old one", "Ordinary edited"));
    expect(synchronize).toHaveBeenCalledOnce();
  });

  it("serializes validated review proofs and rejects lossy/cross-page snapshots before dispatch", () => {
    const { state } = setup();
    const tr = appendNoteReviewBaseline(state(), state().tr, document("New"), { origin: "review", requestId: "proof" });
    const encoded = tr.steps[0]!.toJSON();
    const restored = Step.fromJSON(state().schema, encoded);
    expect(restored.toJSON()).toEqual(encoded);
    expect(restored.invert(state().doc).toJSON()).toMatchObject({ before: document("New"), after: document(), requestId: "proof", direction: "undo" });
    expect(() => Step.fromJSON(state().schema, { ...encoded, direction: "apply" })).toThrow();
    expect(() => Step.fromJSON(state().schema, { ...encoded, after: { ...document(), pageId: "other" } })).toThrow();
    expect(() => appendNoteReviewBaseline(state(), state().tr, { ...document(), surprise: "discarded" } as NoteDocumentV1, { origin: "review" })).toThrow();
    expect(() => appendNoteReviewBaseline(state(), state().tr, { ...document(), pageId: notePageId("other") }, { origin: "review" })).toThrow();
  });
});
