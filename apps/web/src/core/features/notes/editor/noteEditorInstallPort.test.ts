// @vitest-environment happy-dom
import { BlockNoteEditor } from "@blocknote/core";
import { afterEach, describe, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { blockNoteToNoteDocument } from "./blockNoteAdapter";
import { createNoteEditorInstallPort } from "./noteEditorInstallPort";
import type { NoteEditorBlock, NoteEditorInstance } from "./noteEditorSchema";

const pageId = notePageId("page-1");
const cleanup: (() => void)[] = [];
afterEach(() => { cleanup.splice(0).forEach(dispose => dispose()); });
function setup() {
  const editor = BlockNoteEditor.create({ initialContent: [
    { id: "one", type: "paragraph", content: "First", children: [{ id: "child", type: "paragraph", content: "Child" }] },
    { id: "two", type: "paragraph", content: "Second" },
  ] });
  const host = document.createElement("div");
  document.body.append(host);
  editor.mount(host);
  cleanup.push(() => { editor.unmount(); host.remove(); });
  const read = () => blockNoteToNoteDocument(pageId, editor.document as NoteEditorBlock[]);
  const canInstall = vi.fn(() => true);
  const port = createNoteEditorInstallPort({ editor: editor as unknown as NoteEditorInstance, pageId, canInstall });
  return { editor, read, port, canInstall };
}
function rewrite(source: NoteDocumentV1): NoteDocumentV1 {
  const value = structuredClone(source);
  return { ...value, blocks: value.blocks.map(block => ({ ...block,
    content: { kind: "inline", items: [{ type: "text", text: `${block.id} rewritten`, styles: [] }] },
  })) };
}

describe("native note result installation in real BlockNote", () => {
  it("installs structure under a save fence, preserves untouched blocks, and supports one-step editor undo", () => {
    const { editor, read, port } = setup();
    const baseline = read();
    const inserted = { ...baseline.blocks[1]!, id: noteBlockId("inserted"), content: { kind: "inline" as const, items: [{ type: "text" as const, text: "Inserted", styles: [] }] } };
    const candidate = { ...baseline, blocks: [baseline.blocks[0]!, inserted, baseline.blocks[1]!] };
    editor.isEditable = false;
    port.prepareStructure(baseline, candidate)();
    expect(read()).toEqual(candidate);
    editor.isEditable = true; expect(editor.undo()).toBe(true); expect(read()).toEqual(baseline);
  });
  it("projects known defaults and keeps uneditable sidecars out of DOM comparison", () => {
    const { read, port } = setup();
    const baseline = read();
    const native = { ...baseline, blocks: baseline.blocks.map(block => ({ ...block, props: { futureMetadata: "retained" } })) };
    const candidate = { ...native, blocks: [...native.blocks].reverse() };
    port.prepareStructure(native, candidate)();
    expect(read().blocks.map(block => block.id)).toEqual(candidate.blocks.map(block => block.id));
  });
  it("groups all edits and isolates history from both neighboring manual edits", () => {
    const { editor, read, port } = setup();
    editor.updateBlock("one", { content: "Manual before" });
    const baseline = read(), candidate = rewrite(baseline);
    const install = port.prepare(baseline, candidate);
    expect(install()).toBeUndefined();
    expect(read()).toEqual(candidate);
    editor.updateBlock("one", { content: "Manual after" });
    expect(editor.undo()).toBe(true);
    expect(read()).toEqual(candidate);
    expect(editor.undo()).toBe(true);
    expect(read()).toEqual(baseline);
    expect(editor.undo()).toBe(true);
    expect(read().blocks[0]?.content).toEqual({ kind: "inline", items: [{ type: "text", text: "First", styles: [] }] });
    expect(editor.redo()).toBe(true);
    expect(read()).toEqual(baseline);
    expect(editor.redo()).toBe(true);
    expect(read()).toEqual(candidate);
    expect(() => install()).toThrow();
  });

  it("accepts only known omitted defaults, preserving nested IDs and other content", () => {
    const { read, port } = setup();
    const baseline = read();
    const legacy: NoteDocumentV1 = { ...baseline, blocks: baseline.blocks.map(block => ({ ...block, props: {} })) };
    const expected = rewrite(baseline);
    port.prepare(legacy, rewrite(legacy))();
    expect(read()).toEqual(expected);
    expect(read().blocks[0]?.children).toEqual(baseline.blocks[0]?.children);
  });

  it("rejects structure, sibling, page and property changes before touching the editor", () => {
    const { read, port } = setup();
    const baseline = read(), candidate = rewrite(baseline);
    for (const invalid of [
      { ...candidate, pageId: notePageId("other") },
      { ...candidate, blocks: candidate.blocks.slice(1) },
      { ...candidate, blocks: [...candidate.blocks].reverse() },
      { ...candidate, blocks: candidate.blocks.map(block => ({ ...block, props: { color: "red" } })) },
      { ...candidate, blocks: candidate.blocks.map(block => ({ ...block, children: [] })) },
      { ...candidate, blocks: candidate.blocks.map(block => ({ ...block, id: noteBlockId("new") })) },
    ]) expect(() => port.prepare(baseline, invalid)).toThrow();
    expect(read()).toEqual(baseline);
  });

  it("rejects stale, unavailable and disposed installations without overriding new input", () => {
    const { editor, read, port, canInstall } = setup();
    const baseline = read(), candidate = rewrite(baseline);
    const install = port.prepare(baseline, candidate);
    editor.updateBlock("child", { content: "New child input" });
    const latest = read();
    expect(() => install()).toThrow();
    expect(read()).toEqual(latest);
    expect(() => port.prepare(baseline, candidate)).toThrow();
    canInstall.mockReturnValue(false); // IME, asset import, or host operation.
    expect(() => port.prepare(latest, rewrite(latest))).toThrow();
    canInstall.mockReturnValue(true);
    const prepared = port.prepare(latest, rewrite(latest));
    port.dispose();
    expect(() => prepared()).toThrow();
    expect(read()).toEqual(latest);
  });

  it("suppresses only synchronous installation notifications; undo remains ordinary editing", () => {
    const { editor, read, port } = setup();
    const notifications: boolean[] = [];
    const unsubscribe = editor.onChange(() => { notifications.push(port.isInstalling); });
    const baseline = read();
    port.prepare(baseline, rewrite(baseline))();
    expect(notifications).toEqual([true]);
    editor.undo();
    expect(notifications).toEqual([true, false]);
    expect(port.isInstalling).toBe(false);
    unsubscribe();
  });

  it("freezes proposed input and keeps identically named blocks in other instances isolated", () => {
    const left = setup(), right = setup();
    const baseline = left.read(), candidate = rewrite(baseline);
    const install = left.port.prepare(baseline, candidate);
    const expected = structuredClone(candidate);
    // A caller mutating its request object cannot alter the prepared editor work.
    Object.assign(candidate, { blocks: [] });
    install();
    expect(left.read()).toEqual(expected);
    expect(right.read()).toEqual(baseline);
    right.editor.updateBlock("one", { content: "Other session" });
    expect(left.read()).toEqual(expected);
  });
});
