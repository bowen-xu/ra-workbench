// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {TextSelection} from "@tiptap/pm/state";
import {closeHistory} from "@tiptap/pm/history";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {NoteEditorSurface} from "./NoteEditorSurface";
import {noteEditorSchema, type NoteEditorInstance} from "../../features/notes/editor/noteEditorSchema";
import {writingNoteSchema} from "../../../../../../plugins/writing_projects/src/features/writing/notes/writingNoteSchema";
import {blockEditorDefaults} from "./defaults";
import {BlockSetSelection} from "./BlockSetSelection";
import {captionBlockId} from "./captionBlockProjection";
import {readClipboardText} from "../../runtime/platformClipboard";
import {notify} from "../../services/notifications";

vi.mock("../../runtime/platformClipboard", () => ({readClipboardText: vi.fn()}));
vi.mock("../../services/notifications", () => ({notify: {error: vi.fn(), info: vi.fn()}}));
vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
beforeEach(() => {vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true); vi.mocked(readClipboardText).mockReset(); vi.mocked(notify.error).mockClear(); vi.mocked(notify.info).mockClear();});
afterEach(async () => {for (const cleanup of cleanups.splice(0)) await cleanup(); vi.unstubAllGlobals();});

async function fixture(kind: "notes" | "writing" = "notes", readOnly = false) {
  const editor = BlockNoteEditor.create({...blockEditorDefaults, schema: kind === "notes" ? noteEditorSchema : writingNoteSchema,
    initialContent: [{id:"body", type:"paragraph", content:"before old after"}],
  }) as unknown as NoteEditorInstance;
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  await act(async () => root.render(<NoteEditorSurface editor={editor} onChange={() => {}} readOnly={readOnly}/>));
  await act(async () => {editor.setTextCursorPosition("body", "start"); editor.focus();});
  const release = async () => {await act(async () => root.unmount()); host.remove();}; cleanups.push(release);
  const press = async (mod: "metaKey" | "ctrlKey" = "metaKey", shiftKey = true) => {
    const event = new KeyboardEvent("keydown", {bubbles:true, cancelable:true, key:"v", code:"KeyV", [mod]:true, shiftKey});
    await act(async () => editor.prosemirrorView.dom.dispatchEvent(event));
    return event;
  };
  return {editor, host, press};
}

it.each(["notes", "writing"] as const)("%s pastes raw text via the shared shortcut, replaces selection and supports undo", async kind => {
  const f = await fixture(kind), view = f.editor.prosemirrorView;
  const raw = "# heading\n**bold** [link](https://example.org)\n- item\n`code` $x_1$\n\\(x_i\\)\n\\[\na\n=\nb\n\\]";
  vi.mocked(readClipboardText).mockResolvedValue(raw);
  const start=view.state.selection.from;
  await act(async () => view.dispatch(closeHistory(view.state.tr.setSelection(TextSelection.create(view.state.doc, start+7, start+10)))));
  const before = JSON.stringify(f.editor.document);
  expect((await f.press()).defaultPrevented).toBe(true);
  expect(readClipboardText).toHaveBeenCalledOnce();
  expect(view.state.doc.textBetween(0, view.state.doc.content.size, "\n")).toBe(`before ${raw} after`);
  expect(f.editor.document.every(block => block.type === "paragraph")).toBe(true);
  const nodes: string[] = []; view.state.doc.descendants(node => {if (node.isInline) {expect(node.marks).toEqual([]); nodes.push(node.type.name);}});
  expect(nodes.every(type => type === "text")).toBe(true);
  await act(async () => f.editor.undo());
  expect(JSON.stringify(f.editor.document)).toBe(before);
  await act(async () => f.editor.redo());
  expect(view.state.doc.textContent).toContain("**bold**");
});

it("supports Ctrl+Shift+V and leaves ordinary Cmd+V to the rich clipboard pipeline", async () => {
  const f = await fixture(); vi.mocked(readClipboardText).mockResolvedValue("raw");
  expect((await f.press("metaKey", false)).defaultPrevented).toBe(false);
  expect(readClipboardText).not.toHaveBeenCalled();
  expect((await f.press("ctrlKey")).defaultPrevented).toBe(true);
  expect(f.editor.prosemirrorState.doc.textContent).toContain("raw");
});

it.each(["selection", "document", "readonly", "unmount"] as const)("discards a late clipboard reply after %s changes", async change => {
  const f = await fixture(); let complete!: (text: string) => void;
  vi.mocked(readClipboardText).mockImplementation(() => new Promise(resolve => {complete=resolve;}));
  await f.press();
  if (change === "unmount") {await cleanups.pop()!();}
  else await act(async () => {
    const view=f.editor.prosemirrorView;
    if (change === "selection") f.editor.setTextCursorPosition("body", "end");
    if (change === "document") view.dispatch(view.state.tr.insertText("new"));
    if (change === "readonly") f.editor.isEditable=false;
  });
  const before=JSON.stringify(f.editor.document);
  await act(async () => complete("must not insert"));
  expect(JSON.stringify(f.editor.document)).toBe(before);
});

it("does not read in read-only mode, delete a selection for empty text or fall back on read failure", async () => {
  const readonly=await fixture("notes",true); await readonly.press(); expect(readClipboardText).not.toHaveBeenCalled();
  const f=await fixture(), before=JSON.stringify(f.editor.document);
  vi.mocked(readClipboardText).mockResolvedValue(""); await f.press(); expect(JSON.stringify(f.editor.document)).toBe(before);
  vi.mocked(readClipboardText).mockRejectedValue(new Error("clipboard denied")); await f.press();
  expect(JSON.stringify(f.editor.document)).toBe(before);
  expect(notify.error).toHaveBeenCalledWith("NOTE_PASTE_TEXT_FAILED：clipboard denied");
});

it("only applies the latest explicit paste request", async () => {
  const f=await fixture(); const complete: ((text:string)=>void)[]=[];
  vi.mocked(readClipboardText).mockImplementation(() => new Promise(resolve => complete.push(resolve)));
  await f.press(); await f.press();
  await act(async () => complete[1]!("latest")); await act(async () => complete[0]!("stale"));
  expect(f.editor.prosemirrorState.doc.textContent).toContain("latest");
  expect(f.editor.prosemirrorState.doc.textContent).not.toContain("stale");
});

it.each(["title", "caption", "code", "cell"] as const)("keeps native %s constraints while inserting literal text", async target => {
  const f=await fixture();
  await act(async () => f.editor.insertBlocks([
    {id:"title",type:"documentTitle",content:"title"},
    {id:"code",type:"codeBlock",content:"code"},
    {id:"table",type:"table",props:{caption:"caption"},content:{type:"tableContent",rows:[{cells:["cell"]}]}},
  ],"body","after"));
  const id=target==="caption"?captionBlockId("table"):target==="cell"?"table":target;
  await act(async () => {f.editor.setTextCursorPosition(id,"start");f.editor.focus();});
  vi.mocked(readClipboardText).mockResolvedValue("**raw**\nnext");
  await f.press();
  let text=""; f.editor.prosemirrorState.doc.descendants(node=>{if(node.attrs.id===id)text=node.textContent;});
  expect(text).toContain(target==="title"?"**raw** next":"**raw**");
  expect(notify.error).not.toHaveBeenCalled();
  if(target==="caption")expect(f.editor.getBlock("table")?.props).toMatchObject({caption:"**raw**\nnextcaption"});
});

it("replaces a native disjoint block selection and retains the intervening block",async () => {
  const f=await fixture();
  await act(async () => f.editor.insertBlocks([{id:"middle",type:"paragraph",content:"keep middle"},{id:"last",type:"paragraph",content:"replace last"}],"body","after"));
  const view=f.editor.prosemirrorView;
  await act(async () => view.dispatch(view.state.tr.setSelection(BlockSetSelection.create(view.state.doc,["body","last"]))));
  vi.mocked(readClipboardText).mockResolvedValue("**literal**"); await f.press();
  expect(view.state.doc.textContent).toContain("**literal**");expect(view.state.doc.textContent).toContain("keep middle");
  expect(view.state.doc.textContent).not.toContain("replace last");expect(view.state.doc.textContent).not.toContain("before old after");
});
