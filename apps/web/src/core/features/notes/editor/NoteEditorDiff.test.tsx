// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { BlockNoteEditor } from "@blocknote/core";
import { afterEach, expect, it, vi } from "vitest";
import { noteBlockId, notePageId, type NoteBlockV1, type NoteDocumentV1 } from "@research-assistant/core/notes";
import { BlockEditor } from "../../../components/block-editor/BlockEditor";
import { noteDocumentToBlockNote } from "./blockNoteAdapter";
import { NoteEditorDiff } from "./NoteEditorDiff";
import { noteEditorSchema, type NoteEditorInstance } from "./noteEditorSchema";
import { createNoteDiffPlugin, type NoteEditorDiffChunk } from "./noteDiffDecorations";

vi.hoisted(() => {
  if (document.doctype === null) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", { configurable: true, value: "CSS1Compat" });
});
const notes = vi.hoisted(() => ({ client: { gateway: { readNoteAsset: async () => [1, 2, 3] } } }));
vi.mock("../NotesClient", () => ({ useNotesClient: () => notes.client, useNotesVersion: () => 0 }));
const cleanup: (() => Promise<void>)[] = [];
afterEach(async () => { for (const dispose of cleanup.splice(0)) await dispose(); vi.unstubAllGlobals(); });
function paragraph(id: string, text: string): NoteBlockV1 {
  return { id: noteBlockId(id), type: "paragraph", props: {}, children: [], content: { kind: "inline", items: [{ type: "text", text, styles: [] }] } };
}
async function setup(blocks: NoteBlockV1[], chunks: readonly NoteEditorDiffChunk[]) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const page: NoteDocumentV1 = { schemaVersion: 1, pageId: notePageId("diff"), blocks };
  const editor = BlockNoteEditor.create({ schema: noteEditorSchema, initialContent: noteDocumentToBlockNote(page) }) as unknown as NoteEditorInstance;
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host), onResolve = vi.fn(), onChange = vi.fn();
  cleanup.push(async () => { await act(async () => root.unmount()); host.remove(); });
  const render = async (next: readonly NoteEditorDiffChunk[], disabled = false, readOnly = false, controlsTarget?: HTMLElement) => {
    await act(async () => root.render(<BlockEditor className="note-editor" editor={editor} onChange={onChange} readOnly={readOnly}
      beforeEditor={<NoteEditorDiff editor={editor} chunks={next} disabled={disabled} onResolve={onResolve}
        {...(controlsTarget ? { controlsTarget } : {})} />} />));
    await act(async () => { await new Promise<void>(resolve => requestAnimationFrame(() => resolve())); });
  };
  await render(chunks);
  return { host, editor, render, onResolve, onChange };
}
function button(host: HTMLElement, label: string): HTMLButtonElement {
  const result = host.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
  if (!result) throw new Error(`Missing ${label}`);
  return result;
}

it("decorates the true BlockNote instance with rich deletions, additions and shared review controls", async () => {
  const old: NoteBlockV1 = { ...paragraph("changed", ""), content: { kind: "inline", items: [
    { type: "text", text: "Original ", styles: ["bold"] },
    { type: "link", href: "https://example.org/method", content: [{ type: "text", text: "method", styles: [] }] },
  ] } };
  const after = paragraph("changed", "Revised method");
  const chunk = { id: "c1", before: [old], after: [after], afterFrom: 0 };
  const { host, editor, render, onResolve, onChange } = await setup([after, paragraph("untouched", "Unchanged")], [chunk]);
  const surface = host.querySelector(".bn-editor");
  expect(host.querySelectorAll(".bn-editor")).toHaveLength(1);
  expect(host.querySelector(".note-diff-deleted-blocks")?.getAttribute("contenteditable")).toBe("false");
  expect(host.querySelector(".note-diff-deleted-blocks")?.getAttribute("data-block-review-anchor-for")).toBe("changed");
  expect(host.querySelector('.bn-block-outer[data-id="changed"]')?.getAttribute("data-block-review-anchor")).toBe("changed");
  expect(host.querySelector(".note-diff-deleted-blocks [data-id]")).toBeNull();
  expect(host.querySelector(".note-diff-deleted-blocks strong")?.textContent).toBe("Original ");
  expect(host.querySelector(".note-diff-deleted-blocks a")?.getAttribute("href")).toBe("https://example.org/method");
  expect(host.querySelector(".note-diff-deleted-text")?.textContent).not.toBe("");
  expect(host.querySelector(".note-diff-added-text")?.textContent).not.toBe("");
  expect(host.querySelector(".writing-diff-control-row")?.textContent).toContain("1/1");
  expect(editor.document[0]?.content).toEqual([{ type: "text", text: "Revised method", styles: {} }]);
  expect(onChange).not.toHaveBeenCalled();
  await act(async () => button(host, "保留第 1 处变更").click());
  expect(onResolve).toHaveBeenLastCalledWith("keep", "c1");
  await act(async () => button(host, "Undo all").click());
  expect(onResolve).toHaveBeenLastCalledWith("undo", undefined);
  await render([]);
  expect(host.querySelector(".bn-editor")).toBe(surface);
  expect(host.querySelector(".writing-diff-control-row")).toBeNull();
  expect(host.querySelector(".note-diff-deleted-blocks")).toBeNull();
  expect(editor.document[0]?.id).toBe("changed");
  await render([chunk], true);
  expect(host.querySelector(".bn-editor")).toBe(surface);
  expect(button(host, "Keep all").disabled).toBe(true);
  expect(button(host, "保留第 1 处变更").disabled).toBe(true);
});

it("anchors pure deletions in place and keeps pure insertions free of empty old widgets", async () => {
  const before = paragraph("deleted", "Removed rich block"), after = paragraph("added", "Inserted block");
  const chunks = [
    { id: "delete", before: [before], after: [], afterFrom: 1 },
    { id: "insert", before: [], after: [after], afterFrom: 2 },
  ];
  const { host, editor, onResolve } = await setup([paragraph("first", "First"), paragraph("middle", "Middle"), after], chunks);
  expect(host.querySelectorAll(".note-diff-deleted-blocks")).toHaveLength(1);
  expect(host.querySelector('.bn-block-outer[data-id="added"]')?.hasAttribute("data-block-review-anchor")).toBe(false);
  expect(host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(2);
  const group = host.querySelector(".bn-editor > .bn-block-group");
  expect(group?.textContent).toMatch(/FirstRemoved rich blockUndoKeepMiddleInserted blockUndoKeep/);
  await act(async () => button(host, "撤销第 1 处变更").click());
  expect(onResolve).toHaveBeenLastCalledWith("undo", "delete");
  await act(async () => { editor.setTextCursorPosition("first", "start"); button(host, "下一处变更").click(); });
  expect(host.querySelector('[aria-label="当前变更位置"]')?.textContent).toBe("1/2");
  await act(async () => button(host, "下一处变更").click());
  expect(host.querySelector('[aria-label="当前变更位置"]')?.textContent).toBe("2/2");
  const last = editor.prosemirrorState.selection.head;
  await act(async () => button(host, "下一处变更").click());
  expect(editor.prosemirrorState.selection.head).toBe(last);
  await act(async () => button(host, "上一处变更").click());
  expect(host.querySelector('[aria-label="当前变更位置"]')?.textContent).toBe("1/2");
  const first = editor.prosemirrorState.selection.head;
  await act(async () => button(host, "上一处变更").click());
  expect(editor.prosemirrorState.selection.head).toBe(first);
});

it("preserves inline formula and nested-block renderers in readonly old content", async () => {
  const child = paragraph("nested", "Nested content");
  const before: NoteBlockV1 = { ...paragraph("formula", ""), children: [child], content: { kind: "inline", items: [
    { type: "text", text: "Energy ", styles: [] }, { type: "inlineFormula", source: "E=mc^2" },
  ] } };
  const after = paragraph("formula", "Changed energy");
  const { host } = await setup([after], [{ id: "math", before: [before], after: [after], afterFrom: 0 }]);
  const old = host.querySelector(".note-diff-deleted-blocks");
  expect(old?.textContent).toContain("Nested content");
  expect(old?.querySelector(".katex")).not.toBeNull();
  expect(old?.querySelector('[aria-label="E=mc^2"]')).not.toBeNull();
});

it("marks a changed inline formula as one complete atom without marking unchanged neighboring text", async () => {
  const before: NoteBlockV1 = { ...paragraph("formula", ""), content: { kind: "inline", items: [
    {type:"text",text:"unchanged prefix ",styles:[]},{type:"inlineFormula",source:"x^2"},{type:"text",text:" unchanged suffix",styles:[]},
  ] } };
  const after: NoteBlockV1 = {...before,content:{kind:"inline",items:[
    {type:"text",text:"unchanged prefix ",styles:[]},{type:"inlineFormula",source:"x^3"},{type:"text",text:" unchanged suffix",styles:[]},
  ]}};
  const {host}=await setup([after],[{id:"formula-atom",before:[before],after:[after],afterFrom:0}]);
  const current=host.querySelector('.note-diff-added-atom');
  const old=host.querySelector('.note-diff-deleted-atom[data-inline-content-type="inlineFormula"]');
  expect(current?.querySelector('[data-inline-content-type="inlineFormula"]')?.getAttribute("data-source")).toBe("x^3");expect(old?.getAttribute("data-source")).toBe("x^2");
  expect(current?.querySelector(".katex")).not.toBeNull();expect(old?.querySelector(".katex")).not.toBeNull();
  expect(host.querySelector('.katex .note-diff-added-text,.katex .note-diff-deleted-text')).toBeNull();
  expect(host.querySelector('.note-diff-added-text,.note-diff-deleted-text')).toBeNull();
});

it("highlights text beside an unchanged formula without marking that atom", async () => {
  const before: NoteBlockV1 = {...paragraph("formula",""),content:{kind:"inline",items:[
    {type:"text",text:"old ",styles:[]},{type:"inlineFormula",source:"x^2"},
  ]}};
  const after: NoteBlockV1 = {...before,content:{kind:"inline",items:[
    {type:"text",text:"new ",styles:[]},{type:"inlineFormula",source:"x^2"},
  ]}};
  const {host}=await setup([after],[{id:"text-beside-atom",before:[before],after:[after],afterFrom:0}]);
  expect(host.querySelector('.note-diff-added-atom,.note-diff-deleted-atom')).toBeNull();
  expect(host.querySelector('.note-diff-added-text')?.textContent).toBe("new");
  expect(host.querySelector('.note-diff-deleted-text')?.textContent).toBe("old");
});

it.each(["changed", "inserted", "deleted", "unchanged"] as const)("marks the complete block formula for %s content without splitting KaTeX", async mode => {
  const before: NoteBlockV1 = { ...paragraph("block-formula", ""), type: "formulaBlock", content: { kind: "formula", source: "E = mc^2" } };
  const after: NoteBlockV1 = mode === "unchanged" ? before : { ...before, content: { kind: "formula", source: "E = 139mc^2" } };
  const originals = mode === "inserted" ? [] : [before];
  const current = mode === "deleted" ? [] : [after];
  const { host, editor, onChange } = await setup([...current, paragraph("neighbor", "Unchanged neighbor")],
    [{ id: "block-formula-review", before: originals, after: current, afterFrom: 0 }]);
  const oldAtom = host.querySelector('.note-diff-deleted-block-atom .note-formula-preview');
  const newAtom = host.querySelector('.note-diff-added-block-atom .note-formula-preview');
  expect(!!oldAtom).toBe(mode === "changed" || mode === "deleted");
  expect(!!newAtom).toBe(mode === "changed" || mode === "inserted");
  if (oldAtom) {
    expect(oldAtom.getAttribute("aria-label")).toBe("E = mc^2");
    expect(oldAtom.querySelector(".katex")).not.toBeNull();
    expect(oldAtom.closest('[contenteditable="false"]')).not.toBeNull();
  }
  if (newAtom) {
    expect(newAtom.getAttribute("aria-label")).toBe("E = 139mc^2");
    expect(newAtom.querySelector(".katex")).not.toBeNull();
    expect(newAtom.closest("button")?.disabled).toBe(false);
  }
  expect(host.querySelector('.katex .note-diff-added-text,.katex .note-diff-deleted-text')).toBeNull();
  expect(host.querySelector('[data-id="neighbor"] .note-diff-added-block-atom')).toBeNull();
  expect(host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(1);
  expect(editor.document.map(block => block.id)).toEqual([...current.map(block => block.id), "neighbor"]);
  expect(onChange).not.toHaveBeenCalled();
});

it("marks a nested block formula without marking unchanged sibling formulas", async () => {
  const formula: NoteBlockV1 = { ...paragraph("nested-formula", ""), type: "formulaBlock", content: { kind: "formula", source: "x^2" } };
  const untouched: NoteBlockV1 = { ...formula, id: noteBlockId("untouched-formula") };
  const before = { ...paragraph("parent", "Unchanged"), children: [formula, untouched] };
  const after = { ...before, children: [{ ...formula, content: { kind: "formula" as const, source: "x^3" } }, untouched] };
  const { host } = await setup([after], [{ id: "nested-formulas", before: [before], after: [after], afterFrom: 0 }]);
  expect(host.querySelectorAll('.note-diff-deleted-block-atom .note-formula-preview')).toHaveLength(1);
  expect(host.querySelectorAll('.note-diff-added-block-atom .note-formula-preview')).toHaveLength(1);
  expect(host.querySelector('.note-diff-deleted-block-atom .note-formula-preview')?.getAttribute("aria-label")).toBe("x^2");
  expect(host.querySelector('.note-diff-added-block-atom .note-formula-preview')?.getAttribute("aria-label")).toBe("x^3");
});

it("constructs deletion widgets from the supplied view without requiring BlockNote's view getter", async () => {
  const editor = BlockNoteEditor.create({ initialContent: [{ id: "mounting", type: "paragraph", content: "After" }] });
  const presentation = { disabled: false, chunks: [{ id: "mounted", before: [paragraph("mounting", "Before")], after: [paragraph("mounting", "After")], afterFrom: 0 }] };
  const host = document.createElement("div"); document.body.append(host);
  editor.mount(host);
  const getter = vi.spyOn(editor, "prosemirrorView", "get").mockImplementation(() => { throw new Error("view not yet available"); });
  cleanup.push(async () => { getter.mockRestore(); editor.unmount(); host.remove(); });
  const plugin = createNoteDiffPlugin(editor as unknown as NoteEditorInstance, presentation, { onResolve: vi.fn(), onSelection: vi.fn() });
  expect(() => editor._tiptapEditor.registerPlugin(plugin.plugin)).not.toThrow();
  await Promise.resolve();
  expect(host.querySelector(".note-diff-deleted-blocks")?.textContent).toBe("Before");
  expect(host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(1);
});

it("keeps the production asynchronous managed-image renderer alive inside deleted snapshots", async () => {
  const before: NoteBlockV1 = { id: noteBlockId("old-image"), type: "image", props: {textAlignment: "right"}, children: [], content: {
    kind: "asset", assetId: "retained-image", displayName: "旧图.png", mimeType: "image/png", caption: "原始图说明",
  } };
  const after = paragraph("new-body", "替代内容");
  const { host, render } = await setup([after], [{ id: "image", before: [before], after: [after], afterFrom: 0 }]);
  const old = host.querySelector(".note-diff-deleted-blocks");
  expect(old?.querySelector(".managed-note-asset img")?.getAttribute("src")).toMatch(/^blob:/u);
  expect(old?.querySelector(".managed-note-asset img")?.getAttribute("alt")).toBe("原始图说明");
  expect(old?.querySelector(".managed-note-asset figcaption")?.textContent).toContain("原始图说明");
  expect(old?.textContent).not.toContain("旧图.png");
  expect(old?.querySelector(".note-image-block")?.getAttribute("data-alignment")).toBe("right");
  expect(old?.querySelector('[aria-label="图片操作"]')).toBeNull();
  expect(host.querySelectorAll(".bn-editor")).toHaveLength(1);
  await render([]);
  expect(host.querySelector(".note-diff-asset-renderer")).toBeNull();
});

it("retains the remaining addition chunk after partial Keep locks and unlocks the same BlockNote instance", async () => {
  const old = paragraph("changed", "Original"), after = paragraph("changed", "Revised"), added = paragraph("added", "Additional");
  const chunks = [
    { id: "modified", before: [old], after: [after], afterFrom: 0 },
    { id: "addition", before: [], after: [added], afterFrom: 2 },
  ];
  const { host, editor, render } = await setup([after, paragraph("stable", "Stable"), added], chunks);
  const dom = host.querySelector(".bn-editor"), document = JSON.stringify(editor.document);
  expect(host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(2);
  await render(chunks, true, true);
  expect(host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(2);
  await render([chunks[1]!], false, false);
  expect(host.querySelector(".bn-editor")).toBe(dom);
  expect(JSON.stringify(editor.document)).toBe(document);
  expect(host.querySelector(".note-diff-deleted-blocks")).toBeNull();
  expect(host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(1);
  expect(host.querySelector(".note-diff-chunk-actions")?.getAttribute("data-note-diff-chunk")).toBe("addition");
  expect(host.querySelectorAll(".note-diff-added-block")).toHaveLength(1);
  expect(host.querySelector(".note-diff-added-block")?.getAttribute("data-id")).toBe("added");
});

it("ports the same controls into the host toolbar while decorations stay in the original editor", async () => {
  const after = paragraph("changed", "Revised"), before = paragraph("changed", "Original");
  const chunk = { id: "portal", before: [before], after: [after], afterFrom: 0 };
  const { host, render, onResolve } = await setup([after], [chunk]);
  const target = document.createElement("div"); document.body.append(target);
  cleanup.push(async () => target.remove());
  const dom = host.querySelector(".bn-editor");
  await render([chunk], false, false, target);
  expect(host.querySelector(".writing-diff-control-row")).toBeNull();
  expect(target.querySelector(".writing-diff-control-row")?.textContent).toContain("1/1");
  expect(host.querySelectorAll(".note-diff-chunk-actions")).toHaveLength(1);
  expect(host.querySelector(".bn-editor")).toBe(dom);
  await act(async () => button(target, "Keep all").click());
  expect(onResolve).toHaveBeenLastCalledWith("keep", undefined);
});

it("marks changed text in the corresponding table cell without marking an identical unchanged cell", async () => {
  const cell = (value: string) => ({ kind: "inline" as const, items: [{ type: "text" as const, text: value, styles: [] }] });
  const before: NoteBlockV1 = { ...paragraph("table", ""), type: "table", content: {
    kind: "table", rows: [[cell("中文"), cell("0.65")], [cell("重复"), cell("0.65")]],
  } };
  const after: NoteBlockV1 = { ...before, content: { kind: "table", rows: [[cell("中文"), cell("0.65")], [cell("重复"), cell("0.72（模拟）")]] } };
  const { host, editor } = await setup([after], [{ id: "table-change", before: [before], after: [after], afterFrom: 0 }]);
  const cells = host.querySelectorAll('.note-diff-added-block table td');
  expect(cells[1]?.querySelector(".note-diff-added-text")).toBeNull();
  expect(cells[3]?.querySelector(".note-diff-added-text")?.textContent).toBe("72（模拟）");
  expect(host.querySelectorAll('.note-diff-added-block .note-diff-added-text')).toHaveLength(1);
  const oldCells = host.querySelectorAll('.note-diff-deleted-blocks table td');
  expect(oldCells[1]?.querySelector(".note-diff-deleted-text")).toBeNull();
  expect(oldCells[3]?.querySelector(".note-diff-deleted-text")?.textContent).toBe("65");
  expect(editor.document).toHaveLength(1);
  expect(editor.document[0]?.content).toMatchObject({ rows: [{ cells: [expect.anything(), expect.anything()] }, { cells: [expect.anything(), expect.anything()] }] });
});

it.each(["codeBlock", "quote"] as const)("embeds old/new review inside one live %s without adding old text to its document", async type => {
  const before = { ...paragraph("embedded", "Original line\nUnchanged line"), type };
  const after = { ...paragraph("embedded", "Revised line\nUnchanged line"), type };
  const chunk = { id: "embedded-change", before: [before], after: [after], afterFrom: 0 };
  const { host, editor, render, onResolve } = await setup([after], [chunk]);
  const selector = type === "codeBlock" ? "pre > code" : "blockquote";
  const container = host.querySelector(selector), surface = host.querySelector(".bn-editor");
  expect(host.querySelectorAll(selector)).toHaveLength(1);
  expect(container?.querySelector(".note-diff-deleted-content")?.textContent).toContain("Original line");
  expect(container?.querySelector(".note-diff-deleted-content")?.textContent).toContain("Unchanged line");
  expect(container?.querySelector(".note-diff-added-text")?.textContent).not.toBe("");
  expect(host.querySelector(".note-diff-deleted-blocks")).toBeNull();
  expect(JSON.stringify(editor.document)).toContain("Revised line");
  expect(JSON.stringify(editor.document)).not.toContain("Original line");
  await act(async () => {
    editor.setTextCursorPosition("embedded", "end");
    editor._tiptapEditor.commands.insertContent(" user input");
  });
  expect(JSON.stringify(editor.document)).toContain("user input");
  expect(JSON.stringify(editor.document)).not.toContain("Original line");
  await act(async () => button(host, "撤销第 1 处变更").click());
  expect(onResolve).toHaveBeenLastCalledWith("undo", "embedded-change");
  await render([]);
  expect(host.querySelector(".note-diff-deleted-content")).toBeNull();
  expect(host.querySelector(".bn-editor")).toBe(surface);
  expect(host.querySelectorAll(selector)).toHaveLength(1);
});

it("uses the same syntax highlighter for live and deleted code while review keeps the editor editable", async () => {
  const before: NoteBlockV1 = { ...paragraph("python", "print(1)\n# old comment"), type: "codeBlock", props: { language: "python" } };
  const after: NoteBlockV1 = { ...before, content: { kind: "inline", items: [{ type: "text", text: "print(3)\nif True: pass", styles: [] }] } };
  const { host, editor, render } = await setup([after], [{ id: "syntax", before: [before], after: [after], afterFrom: 0 }]);
  const old = host.querySelector(".note-diff-deleted-content");
  expect(old?.querySelector(".tok-number")?.textContent).toBe("1");
  expect(old?.querySelector(".tok-comment")?.textContent).toBe("# old comment");
  expect(host.querySelector('code > .tok-number, code > span .tok-number')?.textContent).not.toBeNull();
  expect(Array.from(host.querySelectorAll("code .tok-keyword")).map(node => node.textContent)).toContain("if");
  await render([]);
  expect(host.querySelector(".note-diff-deleted-content")).toBeNull();
  expect(host.querySelector("code .tok-number")?.textContent).toBe("3");
  await act(async () => { editor.updateBlock("python", { props: { language: "text" } }); });
  expect(host.querySelector("code .tok-number")).toBeNull();
  expect(JSON.stringify(editor.document)).toContain("print(3)");
});

it("retains action buttons across content receipts and updates their decision identity in place", async () => {
  const before = paragraph("changed", "Original"), after = paragraph("changed", "Candidate");
  const chunk = { id: "generation-one", before: [before], after: [after], afterFrom: 0 };
  const { host, editor, render, onResolve } = await setup([after], [chunk]);
  const keep = button(host, "保留第 1 处变更"), row = keep.closest(".note-diff-chunk-actions");
  await act(async () => { editor.updateBlock("changed", { content: "Candidate + manual input" }); });
  const next = { ...chunk, id: "generation-two", after: [paragraph("changed", "Candidate + manual input")] };
  await render([next]);
  expect(button(host, "保留第 1 处变更")).toBe(keep);
  expect(keep.closest(".note-diff-chunk-actions")).toBe(row);
  await render([next], true);
  expect(button(host, "保留第 1 处变更")).toBe(keep); expect(keep.disabled).toBe(true);
  await render([next]);
  expect(button(host, "保留第 1 处变更")).toBe(keep); expect(keep.disabled).toBe(false);
  await act(async () => keep.click());
  expect(onResolve).toHaveBeenLastCalledWith("keep", "generation-two");
});
