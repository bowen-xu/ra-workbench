// @vitest-environment happy-dom
import { BlockNoteEditor } from "@blocknote/core";
import { expect, it } from "vitest";
import { noteEditorSchema } from "./noteEditorSchema";

it.each([
  ["> ", "paragraph"],
  ['" ', "quote"],
  ["“ ", "quote"],
] as const)("typing %j uses the Notes input rules", (prefix, expectedType) => {
  const editor = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: [{ id: "first", type: "paragraph" }] });
  const host = document.createElement("div");
  document.body.append(host);
  editor.mount(host);
  try {
    editor.setTextCursorPosition("first", "start");
    const view = editor.prosemirrorView;
    for (const text of prefix) {
      const { from, to } = view.state.selection;
      const handled = view.someProp("handleTextInput", (handler) => handler(view, from, to, text, () => view.state.tr.insertText(text, from, to)));
      if (!handled) view.dispatch(view.state.tr.insertText(text, from, to));
    }
    expect(editor.document[0]?.type).toBe(expectedType);
    if (expectedType === "paragraph") expect(view.state.doc.textContent).toBe(prefix);
    else {
      expect(view.state.doc.textContent).toBe("");
      expect(editor.undo()).toBe(true);
      expect(editor.document[0]?.type).toBe("paragraph");
    }
  } finally {
    editor.unmount();
    host.remove();
  }
});

it("still imports Markdown quotes without changing the typed input rule", async () => {
  const editor = BlockNoteEditor.create({ schema: noteEditorSchema });
  const blocks = await editor.tryParseMarkdownToBlocks("> Imported quote");
  expect(blocks[0]?.type).toBe("quote");
});
