// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {expect, it, vi} from "vitest";
import {baseBlockEditorSchema} from "./baseSchema";
import {NoteEditorSurface} from "./NoteEditorSurface";
import {BlockEditorFormattingToolbar} from "./BlockEditorFormattingToolbar";

it("limits the real formatting selector and native heading shortcuts to four levels", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const editor = BlockNoteEditor.create({schema: baseBlockEditorSchema, initialContent: [{id: "p", type: "paragraph", content: "Heading"}]}) as unknown as typeof baseBlockEditorSchema.BlockNoteEditor;
  try {
    await act(async () => root.render(<NoteEditorSurface editor={editor} onChange={() => undefined} defaultFormattingToolbar={false}>
      <BlockEditorFormattingToolbar />
    </NoteEditorSurface>));
    const select = host.querySelector<HTMLButtonElement>('button[aria-haspopup="listbox"]');
    expect(select).not.toBeNull();
    await act(async () => select!.click());
    const labels = [...document.querySelectorAll('[role="option"]')].map(item => item.textContent);
    expect(labels).toContain("四级标题");
    expect(labels).not.toContain("五级标题"); expect(labels).not.toContain("六级标题");
    const fourth = [...document.querySelectorAll<HTMLElement>('[role="option"]')].find(item => item.textContent === "四级标题");
    await act(async () => fourth!.click());
    expect(editor.getBlock("p")).toMatchObject({type: "heading", props: {level: 4}});
    await act(async () => {
      editor.updateBlock("p", {type: "paragraph"}); editor.setTextCursorPosition("p");
      editor._tiptapEditor.commands.keyboardShortcut("Mod-Alt-4");
    });
    expect(editor.getBlock("p")).toMatchObject({type: "heading", props: {level: 4}});
    await act(async () => editor._tiptapEditor.commands.keyboardShortcut("Mod-Alt-5"));
    expect(editor.getBlock("p")).toMatchObject({type: "heading", props: {level: 4}});
    await act(async () => editor._tiptapEditor.commands.keyboardShortcut("Mod-Alt-6"));
    expect(editor.getBlock("p")).toMatchObject({type: "heading", props: {level: 4}});
  } finally { await act(async () => root.unmount()); host.remove(); vi.unstubAllGlobals(); }
});
