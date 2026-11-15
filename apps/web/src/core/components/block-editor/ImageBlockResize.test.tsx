// @vitest-environment happy-dom
import {act} from "react";
import {createRoot} from "react-dom/client";
import {BlockNoteEditor} from "@blocknote/core";
import {afterEach, expect, it, vi} from "vitest";
import {blocksToEditor, editorToBlocks} from "@research-assistant/core/notes";
import {NotesProvider} from "../../features/notes/NotesClient";
import {noteRendererGateway} from "../../../app/previews/hbs-adapters/noteGateway";
import {noteEditorSchema, type NoteEditorInstance} from "../../features/notes/editor/noteEditorSchema";
import {writingNoteSchema, WritingNoteOwner} from "../../../../../../plugins/writing_projects/src/features/writing/notes/writingNoteSchema";
import {NoteEditorSurface} from "./NoteEditorSurface";

vi.mock("../../../../../../plugins/writing_projects/src/features/writing/notes/writingNoteAssets", () => ({
  readWritingNoteAsset: async () => Uint8Array.of(1), exportWritingNoteAsset: vi.fn(),
}));
vi.hoisted(() => {
  if (!document.doctype) document.insertBefore(document.implementation.createDocumentType("html", "", ""), document.documentElement);
  Object.defineProperty(document, "compatMode", {configurable: true, value: "CSS1Compat"});
});
const cleanups: (() => Promise<void>)[] = [];
afterEach(async () => {
  for (const cleanup of cleanups.splice(0)) await cleanup();
  vi.restoreAllMocks(); vi.unstubAllGlobals();
});
async function setup(kind: "notes" | "writing", alignment: "left" | "center" | "right" = "left", readOnly = false, naturalSize = false) {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.spyOn(URL, "createObjectURL").mockReturnValue("blob:resize-image");
  vi.spyOn(URL, "revokeObjectURL").mockImplementation(() => undefined);
  let available = 800;
  const observers = new Set<() => void>();
  vi.stubGlobal("ResizeObserver", class {
    constructor(private readonly callback: () => void) {observers.add(callback);}
    observe() {}
    unobserve() {}
    disconnect() {observers.delete(this.callback);}
  });
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(function(this: HTMLElement) {
    if (this.matches(".note-image-resizable")) return Number.parseFloat(this.style.width) || 320;
    return this.matches(".bn-block-content, .bn-block-group, .react-renderer") ? available : 0;
  });
  const captured = new Set<number>();
  vi.spyOn(HTMLElement.prototype, "setPointerCapture").mockImplementation(id => {captured.add(id);});
  vi.spyOn(HTMLElement.prototype, "hasPointerCapture").mockImplementation(id => captured.has(id));
  vi.spyOn(HTMLElement.prototype, "releasePointerCapture").mockImplementation(id => {captured.delete(id);});
  const gateway = new Proxy(noteRendererGateway, {get(target, key) {
    return key === "readNoteAsset" ? async () => Uint8Array.of(1) : Reflect.get(target, key);
  }});
  const host = document.createElement("div"); document.body.append(host); const root = createRoot(host);
  const editor = BlockNoteEditor.create({schema: kind === "notes" ? noteEditorSchema : writingNoteSchema,
    initialContent: [{id: "image", type: "managedImage", props: {
      assetId: "a".repeat(64), displayName: "internal-image.png", mimeType: "image/png", caption: "图 1：算法流程",
      textAlignment: alignment, previewWidth: naturalSize ? 0 : 320,
    }}],
  }) as unknown as NoteEditorInstance;
  const changed = vi.fn();
  const render = async (locked = false) => {
    await act(async () => root.render(<NotesProvider gateway={gateway}><WritingNoteOwner.Provider value="manuscript">
      <NoteEditorSurface editor={editor} readOnly={locked} onChange={changed}/>
    </WritingNoteOwner.Provider></NotesProvider>));
  };
  cleanups.push(async () => {await act(async () => root.unmount()); host.remove();});
  await render(readOnly);
  await vi.waitFor(() => expect(host.querySelector("img")?.getAttribute("src")).toBe("blob:resize-image"));
  changed.mockClear();
  const update = vi.spyOn(editor, "updateBlock");
  const wrapper = () => host.querySelector<HTMLElement>(".note-image-resizable")!;
  let handle: HTMLElement, x = 100;
  const pointer = async (type: string, clientX = x, pointerType = "mouse") => {
    x = clientX;
    await act(async () => handle.dispatchEvent(new PointerEvent(type, {clientX, pointerId: 1, pointerType, isPrimary: true, bubbles: true, cancelable: true})));
  };
  const start = async (side: "left" | "right" = "right", pointerType = "mouse") => {
    handle = host.querySelector<HTMLElement>(`.note-image-resize-handle[data-side="${side}"]`)!;
    await pointer("pointerdown", 100, pointerType);
  };
  const resize = async (next: number) => {
    const prior = wrapper().clientWidth;
    await start(); await pointer("pointermove", 100 + available * next / 100 - prior); await pointer("pointerup");
  };
  const bounds = async (next: number) => {available = next; await act(async () => {for (const refresh of observers) refresh();});};
  return {host, editor, update, changed, render, wrapper, start, pointer, resize, bounds};
}

for (const kind of ["notes", "writing"] as const) {
  it.each(["left", "right"] as const)(`snaps live %s dragging to percentages and commits once in ${kind}`, async side => {
    const f = await setup(kind), before = structuredClone(f.editor.getBlock("image"));
    await f.start(side); await f.pointer("pointermove", side === "left" ? 70 : 130);
    expect(f.wrapper().style.width).toBe("360px");
    expect(f.update).not.toHaveBeenCalled(); expect(f.changed).not.toHaveBeenCalled();
    expect(f.editor.getBlock("image")).toEqual(before);
    await f.pointer("pointerup");
    expect(f.update).toHaveBeenCalledExactlyOnceWith("image", {props: {previewScale: 45, previewWidth: 0}});
    const portable = editorToBlocks(f.editor.document)[0]!;
    expect(portable.props).toEqual({textAlignment: "left", previewScale: 45});
    expect(portable.content).toEqual({kind: "asset", assetId: "a".repeat(64), displayName: "internal-image.png", mimeType: "image/png", caption: "图 1：算法流程"});
    expect(blocksToEditor(JSON.parse(JSON.stringify([portable])))[0]?.props).toMatchObject({previewScale: 45});
  });
  it(`offers all twenty levels including widths below the old 64px minimum in ${kind}`, async () => {
    const f = await setup(kind);
    for (let scale = 5; scale <= 100; scale += 5) {
      await f.resize(scale);
      expect(f.wrapper().style.width).toBe(`${800 * scale / 100}px`);
      expect(f.editor.getBlock("image")?.props).toMatchObject({previewScale: scale, previewWidth: 0});
    }
    await f.resize(-500); expect(f.wrapper().style.width).toBe("40px");
    await f.resize(500); expect(f.wrapper().style.width).toBe("800px");
  });
  it(`keeps the percentage responsive and restores legacy sizing via undo in ${kind}`, async () => {
    const f = await setup(kind);
    await f.resize(50); expect(f.wrapper().style.width).toBe("400px");
    await f.bounds(600); expect(f.wrapper().style.width).toBe("300px");
    await act(async () => {expect(f.editor.undo()).toBe(true);});
    expect(f.wrapper().style.width).toBe("320px");
    await act(async () => {expect(f.editor.redo()).toBe(true);});
    expect(f.wrapper().style.width).toBe("300px");
  });
  it(`starts another gesture from the target level while the preceding visual step is still animating in ${kind}`, async () => {
    const f = await setup(kind);
    await f.resize(50);
    expect(f.wrapper().style.width).toBe("400px");
    // Layout is still approaching 400px, while the stored level is already 50%.
    vi.spyOn(f.wrapper(), "clientWidth", "get").mockReturnValue(350);
    await f.start(); await f.pointer("pointermove", 140); await f.pointer("pointerup");
    expect(f.wrapper().style.width).toBe("440px");
    expect(f.editor.getBlock("image")?.props).toMatchObject({previewScale: 55});
  });
  it(`gives a natural-size legacy image a transition baseline without committing it on pointer down in ${kind}`, async () => {
    const f = await setup(kind, "left", false, true);
    expect(f.wrapper().style.width).toBe("fit-content");
    await f.start(); expect(f.wrapper().style.width).toBe("320px"); expect(f.update).not.toHaveBeenCalled();
    await f.pointer("pointercancel"); expect(f.wrapper().style.width).toBe("fit-content");
    await f.start(); await f.pointer("pointermove", 140); expect(f.wrapper().style.width).toBe("360px");
    await f.pointer("pointerup"); expect(f.editor.getBlock("image")?.props).toMatchObject({previewScale: 45});
  });
  it(`preserves centered symmetric resizing and touch capture in ${kind}`, async () => {
    const f = await setup(kind, "center");
    await f.start("right", "touch"); await f.pointer("pointermove", 130, "touch");
    expect(f.wrapper().style.width).toBe("400px");
    await f.pointer("pointerup", 130, "touch");
    expect(f.editor.getBlock("image")?.props).toMatchObject({previewScale: 50});
  });
  it(`supports keyboard steps without modifying caption or resource in ${kind}`, async () => {
    const f = await setup(kind);
    const handle = f.host.querySelector<HTMLElement>('[role="slider"]')!;
    for (const key of ["End", "ArrowLeft", "Home"]) {
      await act(async () => handle.dispatchEvent(new KeyboardEvent("keydown", {key, bubbles: true, cancelable: true})));
    }
    expect(f.editor.getBlock("image")?.props).toMatchObject({previewScale: 5, caption: "图 1：算法流程", assetId: "a".repeat(64)});
    expect(handle.getAttribute("aria-valuenow")).toBe("5");
  });
  it(`keeps read-only percentage images responsive and cancels when the available width changes in ${kind}`, async () => {
    const f = await setup(kind);
    await f.start(); await f.pointer("pointermove", 180); await f.bounds(600); await f.pointer("pointerup");
    expect(f.update).not.toHaveBeenCalled(); expect(f.wrapper().style.width).toBe("320px");
    await f.resize(50); await f.render(true); await f.bounds(400);
    expect(f.host.querySelector<HTMLElement>(".note-image-block-media")?.style.width).toBe("200px");
    expect(f.host.querySelector('[role="slider"]')).toBeNull();
  });
  it(`cancels pointer loss, Escape and read-only transition without saving in ${kind}`, async () => {
    const f = await setup(kind);
    await f.start(); await f.pointer("pointermove", 240); await f.pointer("pointercancel");
    expect(f.wrapper().style.width).toBe("320px"); expect(f.update).not.toHaveBeenCalled();
    await f.start(); await f.pointer("pointermove", 180); await f.pointer("lostpointercapture");
    expect(f.wrapper().style.width).toBe("320px"); expect(f.update).not.toHaveBeenCalled();
    await f.start(); await f.pointer("pointermove", 220);
    await act(async () => f.host.querySelector<HTMLElement>('[role="slider"]')!.dispatchEvent(new KeyboardEvent("keydown", {key: "Escape", bubbles: true})));
    expect(f.wrapper().style.width).toBe("320px");
    await f.start(); await f.pointer("pointermove", 200); await f.render(true);
    expect(f.host.querySelector('[role="slider"]')).toBeNull();
    await f.pointer("pointerup"); expect(f.update).not.toHaveBeenCalled();
    expect(f.host.querySelector<HTMLElement>(".note-image-block-media")?.style.width).toBe("320px");
  });
}
