// @vitest-environment happy-dom
import { expect, it } from "vitest";
import { noteBlockId, notePageId, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { prepareConflictBlocks } from "./noteConflictMerge";
const block = (id: string, text = id): NoteBlockV1 => ({ id: noteBlockId(id), type: "paragraph", props: {}, content: { kind: "inline", items: [{ type: "text", text, styles: [] }] }, children: [] });
const doc = (...blocks: NoteBlockV1[]): NoteDocumentV1 => ({ schemaVersion: 1, pageId: notePageId("merge-page"), blocks });
it("downgrades repeated and unowned child hierarchy links to ordinary references", () => {
  const child: NoteBlockV1 = { ...block("child"), type: "childPage", content: { kind: "reference", targetKind: "note_page", targetId: "child-page", labelSnapshot: "Child", quoteSnapshot: "" } };
  const result = prepareConflictBlocks([child, child], doc(child));
  expect(result.map(b => b.type)).toEqual(["childPage", "researchObjectReference"]);
  expect(prepareConflictBlocks([child], doc())[0]!.type).toBe("researchObjectReference");
  expect(prepareConflictBlocks([child], doc(child), true)[0]!.type).toBe("researchObjectReference");
});
