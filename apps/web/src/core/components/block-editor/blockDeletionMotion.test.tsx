// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {afterEach, beforeEach, expect, it, vi} from "vitest";
import {closeHistory} from "@tiptap/pm/history";
import {NoteEditorSurface} from "./NoteEditorSurface";
import {blockEditorDefaults} from "./defaults";
import {noteEditorSchema, type NoteEditorInstance} from "../../features/notes/editor/noteEditorSchema";
import {NotesProvider} from "../../features/notes/NotesClient";
import {noteRendererGateway} from "../../../app/previews/hbs-adapters/noteGateway";
import {BlockSetSelection} from "./BlockSetSelection";

vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable:true, value:"CSS1Compat"});
});

let dispose: (() => Promise<void>) | undefined;
const animateDescriptor = Object.getOwnPropertyDescriptor(HTMLElement.prototype, "animate");
let reduced = false;
const frames = new Map<number, FrameRequestCallback>();
let nextFrame = 0;

beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  reduced = false; frames.clear();
  vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => {frames.set(++nextFrame, callback); return nextFrame;});
  vi.spyOn(window, "cancelAnimationFrame").mockImplementation(id => {frames.delete(id);});
  const matchMedia = window.matchMedia.bind(window);
  vi.spyOn(window, "matchMedia").mockImplementation(query => {
    const result = matchMedia(query);
    if (query.includes("prefers-reduced-motion")) Object.defineProperty(result, "matches", {get: () => reduced});
    return result;
  });
});
afterEach(async () => {
  await dispose?.(); dispose = undefined;
  vi.restoreAllMocks(); vi.unstubAllGlobals(); frames.clear();
  if (animateDescriptor) Object.defineProperty(HTMLElement.prototype, "animate", animateDescriptor);
  else Reflect.deleteProperty(HTMLElement.prototype, "animate");
});

async function fixture(nested = false) {
  const viewport = {top: 0};
  const tops = new Map([["title", 0], ["a", 70], ["b", 110], ["child", 140], ["c", 190], ["d", 230]]);
  const visual = new Map<string, number>();
  const motions: {id: string; frames: Keyframe[]; cancel: ReturnType<typeof vi.fn>}[] = [];
  Object.defineProperty(HTMLElement.prototype, "animate", {configurable:true, value: function(this: HTMLElement, keyframes: Keyframe[]) {
    const id = this.dataset.id ?? this.querySelector<HTMLElement>(":scope > .bn-block[data-id]")!.dataset.id!;
    const cancel = vi.fn(() => visual.delete(id));
    motions.push({id, frames:keyframes, cancel});
    return {cancel} as unknown as Animation;
  }});
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function(this: HTMLElement) {
    if (!this.matches(".bn-block-outer")) return new DOMRect(0, viewport.top, 600, 700);
    const id = this.dataset.id ?? this.querySelector<HTMLElement>(":scope > .bn-block[data-id]")?.dataset.id ?? "";
    return new DOMRect(0, viewport.top + (tops.get(id) ?? 0) + (visual.get(id) ?? 0), 600, 35);
  });
  const editor = BlockNoteEditor.create({...blockEditorDefaults, schema:noteEditorSchema, initialContent:[
    {id:"title", type:"documentTitle", content:"标题"},
    {id:"a", type:"paragraph", content:"first"},
    {id:"b", type:"paragraph", content:"middle", ...(nested ? {children:[{id:"child", type:"paragraph" as const, content:"nested"}]} : {})},
    {id:"c", type:"paragraph", content:"third"},
    {id:"d", type:"paragraph", content:"last"},
  ]}) as unknown as NoteEditorInstance;
  const host = document.createElement("div"); document.body.append(host);
  const root = createRoot(host);
  await act(async () => root.render(<NotesProvider gateway={noteRendererGateway}><NoteEditorSurface editor={editor} onChange={() => {}}/></NotesProvider>));
  dispose = () => act(async () => {root.unmount(); host.remove();});
  const flush = () => act(async () => {const pending = [...frames.values()]; frames.clear(); for (const callback of pending) callback(0);});
  await flush(); // Settle the native editor's initial mount frame before input.
  return {editor, host, tops, visual, motions, flush, viewport};
}

it("animates surviving blocks after a first-body deletion without delaying deletion or changing undo", async () => {
  const f = await fixture();
  const original = JSON.stringify(f.editor.document);
  await act(async () => {f.editor.prosemirrorView.dispatch(closeHistory(f.editor.prosemirrorState.tr)); f.editor.removeBlocks(["a"]);});
  expect(f.editor.getBlock("a")).toBeUndefined();
  expect(f.motions).toHaveLength(0); // Native selection scrolling runs first.
  f.tops.set("b", 70); f.tops.set("c", 150); f.tops.set("d", 190);
  await f.flush();
  expect(f.motions.find(motion => motion.id === "b")?.frames).toEqual([{transform:"translateY(40px)"}, {transform:"translateY(0px)"}]);
  await act(async () => {expect(f.editor.undo()).toBe(true);});
  expect(JSON.stringify(f.editor.document)).toBe(original);
});

it("animates middle-block removal and inherits parent motion only once for nested rows", async () => {
  const f = await fixture(true);
  await act(async () => {f.editor.removeBlocks(["a"]);});
  for (const id of ["b", "child", "c", "d"]) f.tops.set(id, f.tops.get(id)! - 40);
  await f.flush();
  expect(f.motions.map(motion => motion.id)).toEqual(["b", "c", "d"]);
  f.motions.length = 0;
  await act(async () => {f.editor.removeBlocks(["c"]);});
  f.tops.set("d", f.tops.get("d")! - 40);
  await f.flush();
  expect(f.motions.map(motion => motion.id)).toEqual(["d"]);
});

it("uses the same motion when Backspace merges a paragraph into its predecessor", async () => {
  const f = await fixture();
  await act(async () => {
    f.editor.setTextCursorPosition("b", "start"); f.editor.focus();
    f.editor.domElement!.dispatchEvent(new KeyboardEvent("keydown", {key:"Backspace", keyCode:8, bubbles:true, cancelable:true}));
  });
  expect(f.editor.getBlock("b")).toBeUndefined();
  expect(f.editor.prosemirrorState.doc.textContent).toContain("firstmiddle");
  f.tops.set("c", 150); f.tops.set("d", 190);
  await f.flush();
  expect(f.motions.map(motion => motion.id)).toEqual(["c", "d"]);
});

it("excludes native selection scrolling from surviving-block displacement", async () => {
  const f = await fixture();
  await act(async () => {f.editor.removeBlocks(["a"]);});
  f.viewport.top = -100; // Native caret scrolling happens before the motion frame.
  f.tops.set("b", 70); f.tops.set("c", 150); f.tops.set("d", 190);
  await f.flush();
  expect(f.motions.find(motion => motion.id === "b")?.frames[0]).toEqual({transform:"translateY(40px)"});
});

it("continues rapid deletions from visible pixels and cancels transforms before IME", async () => {
  const f = await fixture();
  await act(async () => {f.editor.removeBlocks(["a"]);});
  f.tops.set("c", 150); f.tops.set("d", 190);
  await f.flush();
  f.visual.set("d", 20); // The previous 40px motion is halfway complete.
  await act(async () => {f.editor.removeBlocks(["c"]);});
  f.tops.set("d", 150);
  await f.flush();
  expect(f.motions.filter(motion => motion.id === "d").at(-1)?.frames[0]).toEqual({transform:"translateY(60px)"});
  await act(async () => f.editor.domElement!.dispatchEvent(new CompositionEvent("compositionstart", {bubbles:true})));
  expect(f.motions.filter(motion => motion.id === "d").at(-1)?.cancel).toHaveBeenCalled();
});

it("does not animate typing or reduced-motion deletion and releases a queued frame on unmount", async () => {
  const f = await fixture();
  await act(async () => f.editor.updateBlock("a", {content:"ordinary typing"}));
  await f.flush(); expect(f.motions).toHaveLength(0);
  reduced = true;
  await act(async () => {f.editor.removeBlocks(["b"]);});
  await f.flush(); expect(f.motions).toHaveLength(0);
  reduced = false;
  await act(async () => {f.editor.removeBlocks(["c"]);});
  expect(frames.size).toBeGreaterThan(0);
  await dispose?.(); dispose = undefined;
  expect(frames.size).toBe(0);
});

it.each(["text", "image", "enter"] as const)("allows native %s insertion before UniqueID assigns new block identities", async kind => {
  const f = await fixture();
  let html = "<p>pasted first</p><p>pasted second</p>";
  if (kind === "image") await act(async () => {
    f.editor.insertBlocks([{id:"source-image", type:"managedImage", props:{assetId:"asset", previewScale:45, textAlignment:"right"}}], "d", "after");
    const view = f.editor.prosemirrorView;
    view.dispatch(view.state.tr.setSelection(BlockSetSelection.create(view.state.doc, ["source-image"])));
    html = view.serializeForClipboard(view.state.selection.content()).dom.innerHTML;
  });
  const count = f.editor.document.length;
  await act(async () => {
    f.editor.setTextCursorPosition("a", "end");
    f.editor.focus();
    if (kind === "enter") {
      f.editor.domElement!.dispatchEvent(new KeyboardEvent("keydown", {key:"Enter", keyCode:13, bubbles:true, cancelable:true}));
    } else {
      // A real DOM paste first resets copied IDs to null. The deletion motion
      // observer must tolerate that intermediate transaction, with animation
      // available just as it is in a browser (happy-dom omits it by default).
      const paste = new Event("paste", {bubbles:true, cancelable:true});
      Object.defineProperty(paste, "clipboardData", {value:{types:["text/html"], files:[], items:[],
        getData:(type: string) => type === "text/html" ? html : ""}});
      f.editor.domElement!.dispatchEvent(paste);
    }
  });
  const blocks = f.editor.document;
  expect(blocks.length).toBeGreaterThan(count);
  expect(blocks.every(block => !!block.id)).toBe(true);
  expect(new Set(blocks.map(block => block.id)).size).toBe(blocks.length);
  if (kind === "image") {
    const images = blocks.filter(block => block.type === "managedImage");
    expect(images).toHaveLength(2);
    for (const image of images) expect(image.props).toMatchObject({assetId:"asset", previewScale:45, textAlignment:"right"});
  }
  if (kind === "text") expect(f.editor.prosemirrorState.doc.textContent).toContain("pasted second");
  await f.flush();
  expect(f.motions).toHaveLength(0);
});
