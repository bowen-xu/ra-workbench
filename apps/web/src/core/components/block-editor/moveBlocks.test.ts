// @vitest-environment happy-dom
import { BlockNoteEditor } from "@blocknote/core";
import { describe, expect, it } from "vitest";
import { canMoveBlocks, indentSelectedBlocks, moveBlocks } from "./moveBlocks";
import {noteEditorSchema, type NoteEditorInstance} from "../../features/notes/editor/noteEditorSchema";
import {captionBlockId} from "./captionBlockProjection";

describe("host movement constraints", () => {
  it("indents media with its owned caption and validates the whole group's depth", () => {
    const editor = BlockNoteEditor.create({schema: noteEditorSchema, initialContent: [
      {id: "parent", type: "paragraph", content: "Parent"},
      {id: "media", type: "managedImage", props: {caption: "Caption"}, children: [
        {id: captionBlockId("media"), type: "blockCaption", props: {ownerId: "media"}, content: "Caption"},
      ]}, {id: "peer", type: "paragraph", content: "Peer"},
    ]}) as unknown as NoteEditorInstance;
    const original = JSON.stringify(editor.document);
    expect(indentSelectedBlocks(editor, ["media", "peer"], "indent", {maxDepth: 0})).toBe(false);
    expect(JSON.stringify(editor.document)).toBe(original);
    expect(indentSelectedBlocks(editor, ["media", "peer"], "indent", {maxDepth: 1})).toBe(true);
    expect(editor.getBlock("parent")?.children.map(block => block.id)).toEqual(["media", "peer"]);
    expect(editor.getBlock("media")?.children.map(block => block.id)).toEqual([captionBlockId("media")]);
    expect(indentSelectedBlocks(editor, ["media", "peer"], "outdent", {maxDepth: 0})).toBe(true);
    expect(JSON.stringify(editor.document)).toBe(original);
  });

  it("keeps generic nesting separate from a host's container and depth policy", () => {
    const editor = BlockNoteEditor.create({ initialContent: [
      { id: "idea", type: "paragraph", content: "Idea" },
      { id: "section", type: "quote", content: "Section" },
    ] });
    const target = { blockId: "section", placement: "inside" } as const;
    expect(canMoveBlocks(editor.document, ["idea"], target)).toBe(true);
    expect(canMoveBlocks(editor.document, ["idea"], target, { maxDepth: 0 })).toBe(false);
    expect(canMoveBlocks(editor.document, ["idea"], target, { canContain: (block) => block.type !== "quote" })).toBe(false);
    expect(moveBlocks(editor, ["idea"], target, { canNest: () => false })).toBe(false);
    expect(editor.document.map((block) => block.id)).toEqual(["idea", "section"]);
    expect(moveBlocks(editor, ["idea"], target)).toBe(true);
    expect(editor.getBlock("section")?.children.map((block) => block.id)).toEqual(["idea"]);
  });

  it("moves a resource with its attached caption without adding nesting depth or using the caption as a separate destination", () => {
    const editor = BlockNoteEditor.create({schema: noteEditorSchema, initialContent: [
      {id: "before", type: "paragraph", content: "Before"},
      {id: "media", type: "managedImage", props: {caption: "Caption"}, children: [
        {id: captionBlockId("media"), type: "blockCaption", props: {ownerId: "media"}, content: "Caption"},
      ]}, {id: "after", type: "paragraph", content: "After"},
    ]}) as unknown as NoteEditorInstance;
    expect(canMoveBlocks(editor.document, ["media"], {blockId: "after", placement: "after"}, {maxDepth: 0})).toBe(true);
    expect(canMoveBlocks(editor.document, [captionBlockId("media")], {blockId: "after", placement: "after"})).toBe(false);
    expect(moveBlocks(editor, ["before"], {blockId: captionBlockId("media"), placement: "after"})).toBe(true);
    expect(editor.document.map(block => block.id)).toEqual(["media", "before", "after"]);
    expect(editor.getBlock("media")?.children.map(block => block.id)).toEqual([captionBlockId("media")]);
  });
});
