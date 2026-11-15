// @vitest-environment happy-dom
import { BlockNoteEditor } from "@blocknote/core";
import { TextSelection } from "@tiptap/pm/state";
import { expect, it } from "vitest";
import { clearBlockEditorFormatting } from "./clearBlockEditorFormatting";

it("clears mixed styles and undo restores all marks as one step", () => {
  const editor = BlockNoteEditor.create({ initialContent: [{ id: "one", type: "paragraph", content: [
    { type: "text", text: "Bold", styles: { bold: true, underline: true, textColor: "red", backgroundColor: "yellow" } },
    { type: "text", text: " Plain", styles: {} },
  ] }] });
  const host = document.createElement("div"); document.body.append(host); editor.mount(host);
  try {
    let from = 0, to = 0;
    editor.prosemirrorState.doc.descendants((node, pos) => { if (node.type.name === "paragraph" && node.textContent === "Bold Plain") { from = pos + 1; to = from + node.content.size; } });
    editor.transact(tr => tr.setSelection(TextSelection.create(tr.doc, from, to)));
    const before = structuredClone(editor.document);
    clearBlockEditorFormatting(editor);
    expect(editor.document[0]!.content).toEqual([{ type: "text", text: "Bold Plain", styles: {} }]);
    expect(editor.undo()).toBe(true);
    expect(editor.document).toEqual(before);
  } finally { editor.unmount(); host.remove(); }
});
