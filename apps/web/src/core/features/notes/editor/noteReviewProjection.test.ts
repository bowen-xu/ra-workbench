// @vitest-environment happy-dom
import { BlockNoteEditor } from "@blocknote/core";
import { expect, it } from "vitest";
import { noteBlockId, notePageId, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { noteDocumentToBlockNote } from "./blockNoteAdapter";
import { noteEditorSchema } from "./noteEditorSchema";
import { noteReviewDocumentFromState, sameNoteProjection, synchronizeNoteReviewBaseline } from "./noteReviewProjection";

const pageId = notePageId("review-projection");
const paragraph = (id: string, text: string, children: readonly NoteBlockV1[] = []): NoteBlockV1 => ({
  id: noteBlockId(id), type: "paragraph", props: { futureMetadata: `preserved-${id}` }, children,
  content: { kind: "inline", items: [{ type: "text", text, styles: ["bold"] }] },
});
const document = (...blocks: readonly NoteBlockV1[]): NoteDocumentV1 => ({ schemaVersion: 1, pageId, blocks });
const editedText = (block: NoteBlockV1, text: string): NoteBlockV1 => ({ ...block,
  content: { kind: "inline", items: [{ type: "text", text, styles: ["bold"] }] },
});

it("compares actual editor content and known defaults while ignoring hidden native props", () => {
  const original = document(paragraph("one", "Chinese 中文😀"));
  const projected = document({ ...original.blocks[0]!, props: { textAlignment: "left", textColor: "default", backgroundColor: "default" } });
  expect(sameNoteProjection(original, projected)).toBe(true);
  expect(sameNoteProjection(original, document({ ...projected.blocks[0]!, props: { textAlignment: "center" } }))).toBe(false);
  expect(sameNoteProjection(original, document(editedText(projected.blocks[0]!, "Changed")))).toBe(false);
  expect(sameNoteProjection(original, { ...projected, pageId: notePageId("other") })).toBe(false);
});

it("matches native empty paragraphs and table cells without mutating stored empty text nodes", () => {
  const empty = { kind: "inline" as const, items: [{ type: "text" as const, text: "", styles: [] }] };
  const trusted = document(paragraph("empty", ""), { id: noteBlockId("table"), type: "table", props: {}, children: [],
    content: { kind: "table", rows: [[empty, { ...empty, items: [{ type: "text", text: "stable", styles: [] }] }]] } });
  const original = structuredClone(trusted);
  const editor = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: noteDocumentToBlockNote(trusted) });
  const projected = noteReviewDocumentFromState(pageId, editor.prosemirrorState.doc, trusted);
  expect(sameNoteProjection(projected, trusted)).toBe(true);
  expect(trusted).toEqual(original);
});

it("preserves pending old roots while synchronizing ordinary updates, additions and removals", () => {
  const first = paragraph("pending", "Old"), ordinary = paragraph("ordinary", "Before"), removed = paragraph("removed", "Remove me");
  const baseline = document(first, ordinary, removed);
  const current = document(editedText(first, "Agent"), ordinary, removed);
  const added = paragraph("manual-new", "New ordinary block");
  const after = document(editedText(current.blocks[0]!, "Agent + manual"), editedText(ordinary, "User outside"), added);
  const synchronized = synchronizeNoteReviewBaseline(baseline, current, after);
  expect(synchronized).toEqual(document(first, editedText(ordinary, "User outside"), added));
  expect(synchronized.blocks[0]?.id).toBe(first.id);
  expect(synchronized.blocks[0]?.content).toEqual(first.content);
});

it("retains a pending nested root including references, styles and hidden properties", () => {
  const child = paragraph("child", "Old child");
  const link: NoteBlockV1 = { ...paragraph("linked", ""), content: { kind: "inline", items: [
    { type: "mention", targetKind: "paper", targetId: "paper-id", labelSnapshot: "Research" },
    { type: "link", href: "https://example.org/method", content: [{ type: "text", text: "Method", styles: ["italic"] }] },
    { type: "inlineFormula", source: "x^2" },
  ] } };
  const parent = paragraph("parent", "Parent", [child, link]);
  const before = document(parent, paragraph("tail", "Tail"));
  const current = document({ ...parent, children: [editedText(child, "Agent child"), link] }, before.blocks[1]!);
  const after = document({ ...current.blocks[0]!, children: [editedText(child, "Agent child + user"), link] }, editedText(before.blocks[1]!, "Ordinary tail"));
  const synchronized = synchronizeNoteReviewBaseline(before, current, after);
  expect(synchronized.blocks[0]).toEqual(parent);
  expect(synchronized.blocks[1]).toEqual(editedText(before.blocks[1]!, "Ordinary tail"));
});

it("preserves hidden properties recursively when ordinary nested input is projected by the editor", () => {
  const child = paragraph("child", "Before child"), parent = paragraph("parent", "Parent", [child]);
  const before = document(parent, paragraph("pending", "Old"));
  const current = document(parent, editedText(before.blocks[1]!, "Agent"));
  const projectedChild = { ...editedText(child, "Ordinary child edited"), props: {} };
  const projectedParent = { ...parent, props: {}, children: [projectedChild] };
  const synchronized = synchronizeNoteReviewBaseline(before, current, document(projectedParent, current.blocks[1]!));
  expect(synchronized.blocks[0]?.props).toEqual(parent.props);
  expect(synchronized.blocks[0]?.children[0]?.props).toEqual(child.props);
  expect(synchronized.blocks[0]?.children[0]?.content).toEqual(projectedChild.content);
});

it("retains an AI root move in B when a separate ordinary root is edited", () => {
  const a = paragraph("a", "A"), b = paragraph("b", "B"), ordinary = paragraph("ordinary", "Before");
  const before = document(a, b, ordinary), current = document(b, a, ordinary);
  const synchronized = synchronizeNoteReviewBaseline(before, current, document(b, a, editedText(ordinary, "User outside")));
  expect(synchronized).toEqual(document(a, b, editedText(ordinary, "User outside")));
});

it("does not turn suffix roots into pending changes merely because an Agent inserted a block", () => {
  const first = paragraph("first", "First"), tail = paragraph("tail", "Before"), added = paragraph("added", "Agent addition");
  const before = document(first, tail), current = document(first, added, tail);
  const synchronized = synchronizeNoteReviewBaseline(before, current, document(first, added, editedText(tail, "User tail")));
  expect(synchronized).toEqual(document(first, editedText(tail, "User tail")));
});

it("projects nested child-page references from the same trusted note scope without converting them to plain text", () => {
  const childPage: NoteBlockV1 = { id: noteBlockId("child-page"), type: "childPage", props: {}, children: [],
    content: { kind: "reference", targetKind: "note_page", targetId: "target-page", labelSnapshot: "Nested child", quoteSnapshot: "" } };
  const trusted = document(paragraph("parent", "Parent", [childPage]));
  const editor = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: noteDocumentToBlockNote(trusted) });
  const projected = noteReviewDocumentFromState(pageId, editor.prosemirrorState.doc, trusted);
  expect(projected.blocks[0]?.children[0]?.type).toBe("childPage");
  expect(projected.blocks[0]?.children[0]?.content).toEqual(childPage.content);
});

it("keeps manually created roots beside a pending block in the same baseline order and outside the Agent Diff", () => {
  const old = paragraph("pending", "Old"), tail = paragraph("tail", "Tail");
  const current = document(editedText(old, "Agent"), tail);
  const manual = [paragraph("before", ""), paragraph("after-one", "是"), paragraph("after-two", "啊")];
  const after = document(manual[0]!, current.blocks[0]!, manual[1]!, manual[2]!, tail);
  expect(synchronizeNoteReviewBaseline(document(old, tail), current, after)).toEqual(
    document(manual[0]!, old, manual[1]!, manual[2]!, tail));
  // A later keystroke cannot move those already synchronized manual identities
  // in front of the preserved pending block, producing false removed blocks.
  const baseline = synchronizeNoteReviewBaseline(document(old, tail), current, after);
  const updated = document(...after.blocks.map(block => block.id === "after-two" ? editedText(block, "啊！") : block));
  expect(synchronizeNoteReviewBaseline(baseline, after, updated)).toEqual(
    document(manual[0]!, old, manual[1]!, editedText(manual[2]!, "啊！"), tail));
});
