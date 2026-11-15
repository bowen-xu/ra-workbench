// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {afterEach, expect, it, vi} from "vitest";
import {baseBlockEditorSchema} from "./baseSchema";
import {NoteEditorSurface} from "./NoteEditorSurface";
import type {BlockSourceNavigation} from "./BlockSourceNavigation";

const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const clean of cleanups.splice(0)) await clean(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
async function setup(blockId: string, range?: BlockSourceNavigation["range"]) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  const editor = BlockNoteEditor.create({schema: baseBlockEditorSchema, initialContent: [
    {id: "text", type: "paragraph", content: "First\nsecond", children: [{id: "child", type: "paragraph", content: "Nested"}]},
    {id: "divider", type: "divider"},
  ]}) as unknown as typeof baseBlockEditorSchema.BlockNoteEditor;
  const onLocated = vi.fn(), onError = vi.fn(), save = vi.fn();
  await act(async () => root.render(<NoteEditorSurface editor={editor} onChange={save} readOnly
    sourceNavigation={{blockId, token: 1, onLocated, onError, ...(range ? {range} : {})}}/>));
  await act(async () => {await vi.waitFor(() => expect(onLocated.mock.calls.length + onError.mock.calls.length).toBe(1));});
  return {editor, onLocated, onError, save};
}
it.each(["text", "divider", "child"])("locates a mounted %s block in a readonly shared note surface without changing content", async blockId => {
  const {onLocated, onError, save} = await setup(blockId);
  expect(onError).not.toHaveBeenCalled(); expect(onLocated.mock.calls[0]![0].getAnchorRect()).not.toBeNull();
  expect(save).not.toHaveBeenCalled();
});
it("selects a block-relative text range through the shared controller", async () => {
  const {editor, onError} = await setup("text", {startLine: 1, startColumn: 2, endLine: 2, endColumn: 3});
  const {doc, selection} = editor.prosemirrorView.state;
  expect(doc.textBetween(selection.from, selection.to, "\n", "\n")).toBe("irst\nsec");
  expect(onError).not.toHaveBeenCalled();
});
it("reports a deleted target instead of claiming successful navigation", async () => {
  const {onLocated, onError, save} = await setup("missing");
  expect(onLocated).not.toHaveBeenCalled(); expect(onError).toHaveBeenCalledWith(expect.objectContaining({message: expect.stringContaining("不存在")}));
  expect(save).not.toHaveBeenCalled();
});
