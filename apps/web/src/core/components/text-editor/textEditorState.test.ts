// @vitest-environment happy-dom
import { expect, it, vi } from "vitest";
import { EditorView } from "@codemirror/view";
import { getChunks, getOriginalDoc } from "@codemirror/merge";
import { redo, undo } from "@codemirror/commands";
import { Text } from "@codemirror/state";
import { createTextEditorState } from "./textEditorState";
import { resolveTextChunk, resolveTextDiff } from "../editor-diff/textDiffCommands";

it("reviews an incoming Markdown change without losing CRLF or the reversible Keep baseline", () => {
  const before = "# Old\r\n\r\nParagraph 🧪\r\n", after = "# New\r\n\r\nParagraph 🧪\r\n";
  const view = new EditorView({ state: createTextEditorState({ path: "instructions.md", content: after, baseline: before, onChange: () => {} }) });
  try {
    expect(getChunks(view.state)?.chunks.length).toBeGreaterThan(0);
    expect(resolveTextChunk(view, 0, "keep")).toBe(true);
    expect(view.state.doc.toString()).toBe(after); expect(getOriginalDoc(view.state).toString()).toBe(after);
    expect(undo(view)).toBe(true); expect(getOriginalDoc(view.state).toString()).toBe(before);
    expect(resolveTextDiff(view, "undo")).toBe(true); expect(view.state.doc.toString()).toBe(before);
    expect(undo(view)).toBe(true); expect(view.state.doc.toString()).toBe(after);
    expect(redo(view)).toBe(true); expect(view.state.doc.toString()).toBe(before);
  } finally { view.destroy(); }
});
it("normal edits outside a pending diff update both text and baseline", () => {
  const view = new EditorView({ state: createTextEditorState({ path: "text.txt", content: "old", onChange: () => {} }) });
  try {
    view.dispatch({ changes: { from: 3, insert: Text.of(["", "new"]) }, userEvent: "input.type" });
    expect(view.state.doc.toString()).toBe("old\nnew"); expect(getOriginalDoc(view.state).toString()).toBe("old\nnew");
    expect(getChunks(view.state)?.chunks).toHaveLength(0);
    expect(undo(view)).toBe(true); expect(view.state.doc.toString()).toBe("old"); expect(getOriginalDoc(view.state).toString()).toBe("old");
  } finally { view.destroy(); }
});
it("blocks edit and resolution commands for a read-only document", () => {
  const view = new EditorView({ state: createTextEditorState({ path: "builtin.md", content: "new", baseline: "old", readOnly: true, onChange: () => {} }) });
  try { expect(view.contentDOM.contentEditable).toBe("false"); expect(resolveTextDiff(view, "undo")).toBe(false); expect(view.state.doc.toString()).toBe("new"); }
  finally { view.destroy(); }
});
it.each(["config.json", "settings.toml", "workflow.yaml", "script.py", "output.log"])("edits %s as text without an extension gate", path => {
  const source = "first 🧪\nsecond", changes = vi.fn();
  const view = new EditorView({ state: createTextEditorState({ path, content: source, onChange: changes }) });
  try {
    view.dispatch({ changes: { from: source.length, insert: "\nthird" }, userEvent: "input.type" });
    expect(view.state.doc.toString()).toBe(source + "\nthird"); expect(getChunks(view.state)?.chunks).toHaveLength(0);
    expect(changes).toHaveBeenLastCalledWith(source + "\nthird", source + "\nthird");
    expect(undo(view)).toBe(true); expect(view.state.doc.toString()).toBe(source);
  } finally { view.destroy(); }
});
