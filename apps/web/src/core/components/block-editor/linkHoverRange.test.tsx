// @vitest-environment happy-dom
import {act, StrictMode} from "react";
import {createRoot} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {LinkToolbarExtension} from "@blocknote/core/extensions";
import {TextSelection} from "@tiptap/pm/state";
import {afterEach, expect, it, vi} from "vitest";
import {NoteEditorSurface} from "./NoteEditorSurface";
import {blockEditorDefaults} from "./defaults";
import {noteEditorSchema, type NoteEditorInstance, type PartialNoteEditorBlock} from "../../features/notes/editor/noteEditorSchema";
import {writingNoteSchema} from "../../../../../../plugins/writing_projects/src/features/writing/notes/writingNoteSchema";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {for (const cleanup of cleanups.splice(0)) await cleanup(); vi.restoreAllMocks(); vi.unstubAllGlobals();});
const link = (text: string, href = "https://example.org/one") => ({type: "link" as const, href, content: [{type: "text" as const, text, styles: {}}]});
async function fixture(kind: "notes" | "writing", readOnly = false) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  const initialContent: PartialNoteEditorBlock[] = [
    {id: "body", type: "paragraph", content: ["before ", link("3"), " after ", link("long link")]},
    {id: "adjacent", type: "paragraph", content: [link("A"), link("B", "https://example.org/two")]},
    {id: "table", type: "table", content: {type: "tableContent", rows: [{cells: [
      [link("3")], [{type: "link", href: "https://example.org/format", content: [
        {type: "text", text: "bold", styles: {bold: true}}, {type: "text", text: "plain", styles: {}},
      ]}],
    ]}]}},
  ];
  const editor = BlockNoteEditor.create({...blockEditorDefaults, schema: kind === "notes" ? noteEditorSchema : writingNoteSchema, initialContent}) as unknown as NoteEditorInstance;
  const original = editor.getExtension(LinkToolbarExtension)!, host = document.createElement("div");
  document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(<StrictMode><NoteEditorSurface editor={editor} readOnly={readOnly} onChange={() => undefined}/></StrictMode>));
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  return {editor, original, host, root, lookup: editor.getExtension(LinkToolbarExtension)!};
}

for (const kind of ["notes", "writing"] as const) {
  it(`resolves native single-character, adjacent and formatted links in ${kind} without changing document or selection`, async () => {
    const {editor, host, lookup} = await fixture(kind);
    const before = editor.prosemirrorState;
    const anchors = [...host.querySelectorAll<HTMLAnchorElement>("a[href]")];
    expect(anchors.map(anchor => lookup.getLinkAtElement(anchor)?.text)).toEqual(["3", "long link", "A", "B", "3", "boldplain", "boldplain"]);
    // Formatting can split one semantic link into several adjacent anchors.
    const nested = document.createElement("span"), first = anchors[0]!.firstChild!;
    anchors[0]!.replaceChild(nested, first); nested.append(first);
    expect(lookup.getLinkAtElement(nested)?.text).toBe("3");
    for (const anchor of anchors) {
      const hit = lookup.getLinkAtElement(anchor)!;
      expect(hit.mark.attrs.href).toBe(anchor.getAttribute("href"));
      expect(editor.prosemirrorState.doc.textBetween(hit.range.from, hit.range.to)).toContain(anchor.textContent);
    }
    expect(editor.prosemirrorState.doc).toBe(before.doc);
    expect(editor.prosemirrorState.selection.eq(before.selection)).toBe(true);
    expect(lookup.getLinkAtElement(document.createElement("a"))).toBeUndefined();
    expect(lookup.getLinkAtElement(host.querySelector("td p")!)).toBeUndefined();
  });

  it(`preserves the native caret lookup, commands and non-inclusive typing boundary in ${kind}`, async () => {
    const {editor, original, lookup, host} = await fixture(kind);
    expect(lookup).not.toBe(original);
    expect(lookup.getLinkAtSelection).toBe(original.getLinkAtSelection);
    expect(lookup.editLink).toBe(original.editLink);
    expect(lookup.deleteLink).toBe(original.deleteLink);
    expect(editor.prosemirrorState.schema.marks.link?.spec.inclusive).toBe(false);
    const anchor = host.querySelector<HTMLAnchorElement>("a")!;
    const hit = lookup.getLinkAtElement(anchor)!;
    for (const position of [hit.range.from, hit.range.to]) {
      await act(async () => editor.prosemirrorView.dispatch(editor.prosemirrorState.tr.setSelection(TextSelection.create(editor.prosemirrorState.doc, position))));
      expect(lookup.getLinkAtSelection()).toEqual(original.getLinkAtSelection());
      expect(lookup.getLinkAtSelection()).toBeUndefined();
      await act(async () => editor.prosemirrorView.dispatch(editor.prosemirrorState.tr.insertText("x")));
      const $end = editor.prosemirrorState.selection.$from;
      expect($end.nodeBefore?.marks.some(mark => mark.type.name === "link")).toBe(false);
      await act(async () => {editor.undo();});
    }
  });
}

it("restores the original public extension on unmount and keeps read-only toolbars hidden", async () => {
  const {editor, host, root, original, lookup} = await fixture("notes", true);
  expect(lookup.getLinkAtElement(host.querySelector("a")!)?.text).toBe("3");
  expect(editor.isEditable).toBe(false);
  await act(async () => host.querySelector("a")!.dispatchEvent(new MouseEvent("mouseover", {bubbles: true})));
  expect(document.querySelector(".bn-link-toolbar")).toBeNull();
  await act(async () => root.render(null));
  expect(editor.getExtension(LinkToolbarExtension)).toBe(original);
});
