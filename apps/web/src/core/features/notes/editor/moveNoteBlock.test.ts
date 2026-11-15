// @vitest-environment happy-dom
import { BlockNoteEditor } from "@blocknote/core";
import { MAX_NOTE_DEPTH, notePageId } from "@research-assistant/core/notes";
import { describe, expect, it, vi } from "vitest";
import { blockNoteToNoteDocument, noteDocumentToBlockNote } from "./blockNoteAdapter";
import { canMoveBlocks, moveBlocks, type BlockPlacement } from "../../../components/block-editor";
import { noteBlockMovePolicy } from "./noteBlockMovePolicy";
import { noteEditorSchema, type NoteEditorInstance, type PartialNoteEditorBlock } from "./noteEditorSchema";
import type { BlockTreeNode as NoteBlockTreeNode } from "../../../components/block-editor";

// These tests exercise the shared commands with the Notes host policy and codec.
const canMoveNoteBlocks = (blocks: readonly NoteBlockTreeNode[], ids: readonly string[], target: BlockPlacement) => canMoveBlocks(blocks, ids, target, noteBlockMovePolicy);
const canMoveNoteBlock = (blocks: readonly NoteBlockTreeNode[], id: string, target: BlockPlacement) => canMoveNoteBlocks(blocks, [id], target);
const moveNoteBlocks = (editor: NoteEditorInstance, ids: readonly string[], target: BlockPlacement) => moveBlocks(editor, ids, target, noteBlockMovePolicy);
const moveNoteBlock = (editor: NoteEditorInstance, id: string, target: BlockPlacement) => moveNoteBlocks(editor, [id], target);

const tree = [
  { id: "parent", children: [{ id: "child", children: [] }] },
  { id: "quote", children: [] },
  { id: "tail", children: [] },
];

describe("semantic block moves", () => {
  it("rejects self, descendants, absent targets and adjacent no-ops", () => {
    for (const target of ["parent", "child", "missing", "quote"]) {
      expect(canMoveNoteBlock(tree, "parent", { blockId: target, placement: "before" })).toBe(false);
    }
    expect(canMoveNoteBlock(tree, "quote", { blockId: "parent", placement: "after" })).toBe(false);
    expect(canMoveNoteBlock(tree, "missing", { blockId: "parent", placement: "before" })).toBe(false);
  });
  it("allows a child to be detached before its parent, not only sorted among siblings", () => {
    expect(canMoveNoteBlock(tree, "child", { blockId: "parent", placement: "before" })).toBe(true);
  });
  it("uses public commands once, preserving source/subtree IDs and a sibling quote", () => {
    const editor = BlockNoteEditor.create({ initialContent: [
      { id: "parent", type: "paragraph", content: "23", children: [{ id: "child", type: "paragraph", content: "nested" }] },
      { id: "quote", type: "quote", content: "independent quote" },
      { id: "tail", type: "paragraph", content: "tail" },
    ] }) as unknown as NoteEditorInstance;
    expect(moveNoteBlock(editor, "parent", { blockId: "tail", placement: "after" })).toBe(true);
    expect(editor.document.map((block) => block.id)).toEqual(["quote", "tail", "parent"]);
    expect(editor.getBlock("parent")?.children.map((block) => block.id)).toEqual(["child"]);
    expect(moveNoteBlock(editor, "child", { blockId: "parent", placement: "before" })).toBe(true);
    expect(editor.document.map((block) => block.id)).toEqual(["quote", "tail", "child", "parent"]);
    expect(editor.getBlock("parent")?.children).toEqual([]);
  });
});

function createEditor(blocks: PartialNoteEditorBlock[]): NoteEditorInstance {
  // Match the application adapter: 0.52.1's conditional factory return type
  // falls back to the default schema under exactOptionalPropertyTypes.
  return BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: blocks }) as unknown as NoteEditorInstance;
}

describe("multi-block semantic moves", () => {
  it.each([
    { blockId: "b", placement: "after" },
    { blockId: "c", placement: "before" },
  ] as const)("resolves an equivalent gap at $blockId-$placement after removing disjoint selected roots", (target) => {
    const editor = createEditor(["a", "b", "c", "d"].map((id) => ({ id, type: "paragraph", content: id })));
    editor.mount(document.createElement("div"));
    const original = editor.document;
    expect(canMoveNoteBlocks(original, ["c", "a"], target)).toBe(true);
    expect(moveNoteBlocks(editor, ["c", "a"], target)).toBe(true);
    expect(editor.document.map((block) => block.id)).toEqual(["b", "a", "c", "d"]);
    const moved = editor.document;
    expect(editor.undo()).toBe(true);
    expect(editor.document).toEqual(original);
    expect(editor.redo()).toBe(true);
    expect(editor.document).toEqual(moved);
    editor.unmount();
  });

  it("uses a surviving previous sibling for a selected last-block boundary", () => {
    const editor = createEditor(["a", "b", "c"].map((id) => ({ id, type: "paragraph", content: id })));
    expect(moveNoteBlocks(editor, ["a", "c"], { blockId: "c", placement: "after" })).toBe(true);
    expect(editor.document.map((block) => block.id)).toEqual(["b", "a", "c"]);
  });

  it("anchors to the surviving parent when a cross-parent group empties the target sibling group", () => {
    const editor = createEditor([
      { id: "parent", type: "quote", children: [{ id: "a", type: "paragraph", content: "a" }] },
      { id: "b", type: "paragraph", content: "b" },
    ]);
    expect(canMoveNoteBlocks(editor.document, ["a", "b"], { blockId: "a", placement: "before" })).toBe(true);
    expect(moveNoteBlocks(editor, ["a", "b"], { blockId: "a", placement: "before" })).toBe(true);
    expect(editor.document.map((block) => block.id)).toEqual(["parent"]);
    expect(editor.getBlock("parent")?.children.map((block) => block.id)).toEqual(["a", "b"]);
  });

  it("resolves selected anchors only at their sibling level, not inside selected subtrees", () => {
    const editor = createEditor([
      { id: "parent", type: "quote", children: [
        { id: "a", type: "paragraph", content: "a", children: [{ id: "nested", type: "paragraph" }] },
        { id: "b", type: "paragraph", content: "b" },
        { id: "c", type: "paragraph", content: "c" },
      ] },
      { id: "tail", type: "paragraph" },
    ]);
    for (const placement of ["before", "after", "inside"] as const) {
      expect(canMoveNoteBlocks(editor.document, ["a", "c"], { blockId: "nested", placement })).toBe(false);
    }
    expect(canMoveNoteBlocks(editor.document, ["a", "c"], { blockId: "a", placement: "inside" })).toBe(false);
    expect(moveNoteBlocks(editor, ["a", "c"], { blockId: "c", placement: "before" })).toBe(true);
    expect(editor.document.map((block) => block.id)).toEqual(["parent", "tail"]);
    expect(editor.getBlock("parent")?.children.map((block) => block.id)).toEqual(["b", "a", "c"]);
    expect(editor.getBlock("a")?.children.map((block) => block.id)).toEqual(["nested"]);
  });

  it("rejects selected-anchor gaps when the whole group or a single block stays in place", () => {
    const editor = createEditor(["a", "b", "c"].map((id) => ({ id, type: "paragraph", content: id })));
    const remove = vi.spyOn(editor, "removeBlocks");
    for (const placement of ["before", "after"] as const) {
      expect(moveNoteBlocks(editor, ["a", "b", "c"], { blockId: "b", placement })).toBe(false);
      expect(moveNoteBlock(editor, "b", { blockId: "b", placement })).toBe(false);
      expect(moveNoteBlocks(editor, ["a", "b"], { blockId: "b", placement })).toBe(false);
    }
    expect(remove).not.toHaveBeenCalled();
  });

  it("moves disjoint selected roots in document order with children and only one undo step", () => {
    const editor = createEditor([
      { id: "first", type: "paragraph", content: "first", children: [{ id: "child", type: "quote", content: "child" }] },
      { id: "middle", type: "paragraph", content: "middle" },
      { id: "last", type: "paragraph", content: "last" },
      { id: "target", type: "quote", content: "target" },
    ]);
    editor.mount(document.createElement("div"));
    const original = editor.document;
    const focus = vi.spyOn(editor, "focus");
    expect(moveNoteBlocks(editor, ["last", "child", "first", "last"], { blockId: "target", placement: "after" })).toBe(true);
    expect(editor.document.map((block) => block.id)).toEqual(["middle", "target", "first", "last"]);
    expect(editor.getBlock("first")?.children.map((block) => block.id)).toEqual(["child"]);
    expect(focus).toHaveBeenCalledTimes(1);
    const moved = editor.document;
    expect(editor.undo()).toBe(true);
    expect(editor.document).toEqual(original);
    expect(editor.redo()).toBe(true);
    expect(editor.document).toEqual(moved);
    editor.unmount();
  });

  it("detaches selected roots across parents without promoting unselected ancestors or siblings", () => {
    const editor = createEditor([
      { id: "parent-a", type: "paragraph", children: [
        { id: "a", type: "paragraph", content: "a" },
        { id: "a-stays", type: "paragraph", content: "a stays" },
      ] },
      { id: "parent-b", type: "quote", children: [{ id: "b", type: "bulletListItem", content: "b" }] },
      { id: "target", type: "paragraph", content: "target" },
    ]);
    expect(moveNoteBlocks(editor, ["b", "a"], { blockId: "target", placement: "before" })).toBe(true);
    expect(editor.document.map((block) => block.id)).toEqual(["parent-a", "parent-b", "a", "b", "target"]);
    expect(editor.getBlock("parent-a")?.children.map((block) => block.id)).toEqual(["a-stays"]);
    expect(editor.getBlock("parent-b")?.children).toEqual([]);
  });

  it.each(["paragraph", "quote", "bulletListItem", "numberedListItem", "checkListItem", "formulaBlock"] as const)(
    "appends a mixed selection inside an empty %s without changing source identity or content",
    (type) => {
      const editor = createEditor([
        { id: "text", type: "paragraph", content: "text" },
        { id: "formula", type: "formulaBlock", props: { source: "E=mc^2" } },
        { id: "target", type },
      ]);
      expect(moveNoteBlocks(editor, ["formula", "text"], { blockId: "target", placement: "inside" })).toBe(true);
      expect(editor.document.map((block) => block.id)).toEqual(["target"]);
      expect(editor.getBlock("target")?.children.map((block) => block.id)).toEqual(["text", "formula"]);
      const saved = blockNoteToNoteDocument(notePageId("page"), editor.document);
      const reopened = createEditor(noteDocumentToBlockNote(saved));
      expect(reopened.document).toEqual(editor.document);
    },
  );

  it("re-reads an ancestor's remaining children after removal, never duplicating a source", () => {
    const editor = createEditor([
      { id: "parent", type: "quote", content: "parent", children: [
        { id: "child", type: "paragraph", content: "child" },
        { id: "stays", type: "paragraph", content: "stays" },
      ] },
      { id: "other", type: "paragraph", content: "other" },
    ]);
    expect(moveNoteBlocks(editor, ["child", "other"], { blockId: "parent", placement: "inside" })).toBe(true);
    expect(editor.getBlock("parent")?.children.map((block) => block.id)).toEqual(["stays", "child", "other"]);
    expect(editor.document.map((block) => block.id)).toEqual(["parent"]);
    expect(() => blockNoteToNoteDocument(notePageId("page"), editor.document)).not.toThrow();
  });

  it("undoes a grouped inside move and restores both source nesting and existing target children", () => {
    const editor = createEditor([
      { id: "parent", type: "paragraph", children: [{ id: "source", type: "paragraph", content: "source" }] },
      { id: "formula", type: "formulaBlock", props: { source: "x^2" } },
      { id: "target", type: "quote", content: "target", children: [{ id: "existing", type: "bulletListItem", content: "existing" }] },
    ]);
    editor.mount(document.createElement("div"));
    const original = editor.document;
    expect(moveNoteBlocks(editor, ["source", "formula"], { blockId: "target", placement: "inside" })).toBe(true);
    expect(editor.getBlock("target")?.children.map((block) => block.id)).toEqual(["existing", "source", "formula"]);
    const moved = editor.document;
    expect(editor.undo()).toBe(true);
    expect(editor.document).toEqual(original);
    expect(editor.redo()).toBe(true);
    expect(editor.document).toEqual(moved);
    editor.unmount();
  });

  it("does not dispatch the removal when the grouped insertion fails", () => {
    const editor = createEditor([
      { id: "a", type: "paragraph", content: "a" },
      { id: "b", type: "paragraph", content: "b" },
      { id: "target", type: "quote", content: "target" },
    ]);
    const original = editor.document;
    const focus = vi.spyOn(editor, "focus");
    vi.spyOn(editor, "insertBlocks").mockImplementationOnce(() => { throw new Error("insertion failed"); });
    expect(() => moveNoteBlocks(editor, ["a", "b"], { blockId: "target", placement: "after" })).toThrow("insertion failed");
    expect(editor.document).toEqual(original);
    expect(focus).not.toHaveBeenCalled();
  });

  it("rejects an empty/missing selection, selected destinations and descendants of any selected root", () => {
    for (const ids of [[], ["missing"], ["parent", "missing"]]) {
      expect(canMoveNoteBlocks(tree, ids, { blockId: "tail", placement: "inside" })).toBe(false);
    }
    for (const placement of ["before", "after", "inside"] as const) {
      for (const blockId of ["parent", "child", "quote", "missing"]) {
        expect(canMoveNoteBlocks(tree, ["parent", "quote"], { blockId, placement })).toBe(false);
      }
    }
  });

  it("rejects adjacent group and existing child-suffix no-ops without changing focus or history", () => {
    const editor = createEditor([
      { id: "a", type: "paragraph", content: "a" },
      { id: "b", type: "paragraph", content: "b" },
      { id: "target", type: "quote", content: "target", children: [
        { id: "c", type: "paragraph", content: "c" },
        { id: "d", type: "paragraph", content: "d" },
      ] },
    ]);
    const original = editor.document;
    const transact = vi.spyOn(editor, "removeBlocks");
    const focus = vi.spyOn(editor, "focus");
    expect(moveNoteBlocks(editor, ["b", "a"], { blockId: "target", placement: "before" })).toBe(false);
    expect(moveNoteBlocks(editor, ["c", "d"], { blockId: "target", placement: "inside" })).toBe(false);
    expect(transact).not.toHaveBeenCalled();
    expect(focus).not.toHaveBeenCalled();
    expect(editor.document).toEqual(original);
  });

  it("preserves the root-only child-page aggregate boundary while permitting root reorders", () => {
    const editor = createEditor([
      { id: "page", type: "childPage", props: { targetId: "child-page" } },
      { id: "text", type: "paragraph", content: "text" },
      { id: "parent", type: "quote", content: "parent", children: [{ id: "child", type: "paragraph", content: "child" }] },
    ]);
    expect(moveNoteBlock(editor, "page", { blockId: "parent", placement: "inside" })).toBe(false);
    expect(moveNoteBlock(editor, "page", { blockId: "child", placement: "before" })).toBe(false);
    expect(moveNoteBlock(editor, "text", { blockId: "page", placement: "inside" })).toBe(false);
    expect(moveNoteBlocks(editor, ["page", "text"], { blockId: "parent", placement: "after" })).toBe(true);
    expect(editor.document.map((block) => block.id)).toEqual(["parent", "page", "text"]);
    const invalidExistingSubtree = [
      { id: "source", children: [{ id: "nested-page", type: "childPage", children: [] }] },
      { id: "target", children: [{ id: "child", children: [] }] },
    ];
    expect(canMoveNoteBlock(invalidExistingSubtree, "source", { blockId: "target", placement: "inside" })).toBe(false);
    expect(canMoveNoteBlock(invalidExistingSubtree, "source", { blockId: "child", placement: "after" })).toBe(false);
  });

  it("does not move a subtree beyond the existing document nesting safety limit", () => {
    let nested: NoteBlockTreeNode = { id: "deepest", children: [] };
    for (let depth = 0; depth < MAX_NOTE_DEPTH; depth++) nested = { id: `depth-${depth}`, children: [nested] };
    const blocks = [nested, { id: "source", children: [] }];
    expect(canMoveNoteBlock(blocks, "source", { blockId: "deepest", placement: "before" })).toBe(true);
    expect(canMoveNoteBlock(blocks, "source", { blockId: "deepest", placement: "inside" })).toBe(false);
  });
});
