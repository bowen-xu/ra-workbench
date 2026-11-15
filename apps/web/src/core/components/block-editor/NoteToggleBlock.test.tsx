// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {expect, it, vi} from "vitest";
import {BlockNoteEditor} from "@blocknote/core";
import {baseBlockEditorSchema} from "./baseSchema";
import {NoteEditorSurface} from "./NoteEditorSurface";

it("keeps the empty toggle hint outside content until its add command is chosen", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  localStorage.removeItem("toggle-empty-toggle-ui");
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  const editor = BlockNoteEditor.create({schema: baseBlockEditorSchema, initialContent: [
    {id: "empty-toggle-ui", type: "toggleListItem", content: "Title"},
  ]});
  const changed = vi.fn();
  try {
    await act(async () => root.render(<NoteEditorSurface editor={editor} onChange={changed}/>));
    const before = structuredClone(editor.document);
    const toggle = host.querySelector<HTMLButtonElement>(".bn-toggle-button")!;
    changed.mockClear();
    await act(async () => toggle.click());
    expect(editor.document).toEqual(before);
    expect(changed).not.toHaveBeenCalled();
    const add = host.querySelector<HTMLButtonElement>(".bn-toggle-add-block-button")!;
    expect(add).not.toBeNull();
    expect(add.closest('[data-content-type="toggleListItem"]')).not.toBeNull();
    expect(host.querySelectorAll('[data-content-type="paragraph"]')).toHaveLength(0);
    await act(async () => add.click());
    expect(editor.document[0]?.children).toHaveLength(1);
    expect(editor.document[0]?.children[0]?.type).toBe("paragraph");
    expect(host.querySelector(".bn-toggle-add-block-button")).toBeNull();
    await act(async () => {expect(editor.undo()).toBe(true);});
    expect(editor.document).toEqual(before);
    // Native child-count handling closes the toggle when its last child is removed.
    expect(host.querySelector(".bn-toggle-wrapper")?.getAttribute("data-show-children")).toBe("false");
    await act(async () => host.querySelector<HTMLButtonElement>(".bn-toggle-button")!.click());
    expect(host.querySelector(".bn-toggle-add-block-button")).not.toBeNull();
    expect(editor.document).toEqual(before);
  } finally {
    await act(async () => root.unmount()); host.remove();
    localStorage.removeItem("toggle-empty-toggle-ui"); vi.unstubAllGlobals();
  }
});
