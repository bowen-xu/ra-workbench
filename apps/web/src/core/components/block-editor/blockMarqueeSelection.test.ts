// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { isBlockMarqueeStart, startBlockMarqueeSelection } from "./blockMarqueeSelection";

describe("side-whitespace block marquee", () => {
  let editor: HTMLElement;
  let pane: HTMLElement;
  let capture: boolean;
  let cleanup: (() => void) | undefined;
  let frame: FrameRequestCallback | undefined;
  function paint(): void { const pending = frame; frame = undefined; pending?.(0); }
  const callbacks = { onActivate: vi.fn(), onSelect: vi.fn(), onFinish: vi.fn() };
  function pointer(type: string, x: number, y: number, buttons = type === "pointerup" ? 0 : 1, pointerId = 1): PointerEvent {
    return new PointerEvent(type, { bubbles: true, cancelable: true, pointerId, isPrimary: true, pointerType: "mouse", button: 0, buttons, clientX: x, clientY: y });
  }
  beforeEach(() => {
    frame = undefined;
    vi.spyOn(window, "requestAnimationFrame").mockImplementation(callback => { frame = callback; return 1; });
    vi.spyOn(window, "cancelAnimationFrame").mockImplementation(() => { frame = undefined; });
    document.body.innerHTML = '<div class="host-pane" data-block-editor-scroll-container><div class="block-editor"><div class="bn-editor"><div class="bn-block" data-id="parent"><div class="bn-block-content"><p>Parent</p></div><div class="bn-block" data-id="child"><div class="bn-block-content"><p>Child</p></div></div></div><div class="bn-block" data-id="peer"><div class="bn-block-content"><input></div></div></div><button>handle</button></div></div>';
    editor = document.querySelector<HTMLElement>(".bn-editor")!;
    pane = document.querySelector<HTMLElement>(".host-pane")!;
    vi.spyOn(pane, "getBoundingClientRect").mockReturnValue(new DOMRect(20, 50, 680, 450));
    vi.spyOn(editor, "getBoundingClientRect").mockReturnValue(new DOMRect(60, 100, 600, 600));
    const rows = editor.querySelectorAll(".bn-block-content");
    vi.spyOn(rows[0]!, "getBoundingClientRect").mockReturnValue(new DOMRect(110, 120, 500, 26));
    vi.spyOn(rows[1]!, "getBoundingClientRect").mockReturnValue(new DOMRect(134, 150, 476, 26));
    vi.spyOn(rows[2]!, "getBoundingClientRect").mockReturnValue(new DOMRect(110, 180, 500, 26));
    capture = false;
    editor.setPointerCapture = () => { capture = true; };
    editor.hasPointerCapture = () => capture;
    editor.releasePointerCapture = () => { capture = false; };
    vi.clearAllMocks();
  });
  afterEach(() => { cleanup?.(); cleanup = undefined; vi.restoreAllMocks(); document.body.innerHTML = ""; });

  it.each([80, 640])("accepts side whitespace at x=%s without changing content-origin events", (x) => {
    const event = pointer("pointerdown", x, 140);
    pane.dispatchEvent(event);
    expect(isBlockMarqueeStart(editor, event)).toBe(true);
    const text = pointer("pointerdown", 130, 140);
    editor.querySelector("p")!.dispatchEvent(text);
    expect(isBlockMarqueeStart(editor, text)).toBe(false);
  });
  it.each(["button", "input"])("leaves %s controls out of block selection", (tag) => {
    const event = pointer("pointerdown", 80, 140);
    document.querySelector(tag)!.dispatchEvent(event);
    expect(isBlockMarqueeStart(editor, event)).toBe(false);
  });
  it("rejects headers, relationships, other panes and the middle text column", () => {
    for (const [x, y] of [[80, 80], [80, 520], [400, 140], [710, 140]]) {
      const event = pointer("pointerdown", x!, y!);
      pane.dispatchEvent(event);
      expect(isBlockMarqueeStart(editor, event)).toBe(false);
    }
    const outside = pointer("pointerdown", 80, 140);
    document.body.dispatchEvent(outside);
    expect(isBlockMarqueeStart(editor, outside)).toBe(false);
  });
  it("intersects content rows, not their ancestor wrappers, in either direction", () => {
    for (const [startX, startY, endX, endY, expected] of [
      [80, 149, 450, 200, ["child", "peer"]],
      [450, 200, 80, 149, ["child", "peer"]],
      [80, 110, 450, 200, ["parent", "child", "peer"]],
    ] as const) {
      cleanup = startBlockMarqueeSelection(editor, pointer("pointerdown", startX, startY), callbacks);
      window.dispatchEvent(pointer("pointerup", endX, endY));
      expect(callbacks.onSelect).toHaveBeenLastCalledWith(expected);
    }
  });
  it("rejects non-editor content overlapping the editor's visible bounds", () => {
    for (const tag of ["header", "section"]) {
      const content = document.createElement(tag);
      pane.append(content);
      const event = pointer("pointerdown", 30, 140);
      content.dispatchEvent(event);
      expect(isBlockMarqueeStart(editor, event)).toBe(false);
    }
  });
  it("rejects another editor and whitespace beyond this surface in a shared scrolling pane", () => {
    const sibling = document.createElement("div");
    sibling.className = "block-editor";
    pane.append(sibling);
    const overSibling = pointer("pointerdown", 640, 140);
    sibling.dispatchEvent(overSibling);
    expect(isBlockMarqueeStart(editor, overSibling)).toBe(false);
    const outsideSurface = pointer("pointerdown", 680, 140);
    pane.dispatchEvent(outsideSurface);
    expect(isBlockMarqueeStart(editor, outsideSurface)).toBe(false);
  });
  it.each([30, 690])("accepts the host's empty side margin outside the editable root at x=%s", x => {
    const event = pointer("pointerdown", x, 140);
    pane.dispatchEvent(event);
    expect(isBlockMarqueeStart(editor, event)).toBe(true);
  });
  it("accepts bottom whitespace in the text column and the surface's padding", () => {
    const surface = editor.closest<HTMLElement>(".block-editor")!;
    vi.spyOn(pane, "getBoundingClientRect").mockReturnValue(new DOMRect(20, 50, 680, 850));
    vi.spyOn(surface, "getBoundingClientRect").mockReturnValue(new DOMRect(60, 100, 600, 700));
    for (const y of [300, 750]) {
      const event = pointer("pointerdown", 400, y);
      pane.dispatchEvent(event);
      expect(isBlockMarqueeStart(editor, event)).toBe(true);
    }
    const bottom = pointer("pointerdown",400,830);pane.dispatchEvent(bottom);
    expect(isBlockMarqueeStart(editor,bottom)).toBe(true);
    const section=document.createElement("section");pane.append(section);
    const relationships = pointer("pointerdown", 400, 830);
    section.dispatchEvent(relationships);
    expect(isBlockMarqueeStart(editor, relationships)).toBe(false);
  });
  it("keeps clicks below threshold out of selection and cleans up", () => {
    cleanup = startBlockMarqueeSelection(editor, pointer("pointerdown", 80, 110), callbacks);
    window.dispatchEvent(pointer("pointermove", 82, 111));
    window.dispatchEvent(pointer("pointerup", 82, 111));
    expect(callbacks.onActivate).not.toHaveBeenCalled();
    expect(callbacks.onSelect).not.toHaveBeenCalled();
    expect(callbacks.onFinish).toHaveBeenCalledExactlyOnceWith(false);
  });
  it("survives buttons 1→0 and selects from the final release rectangle exactly once", () => {
    cleanup = startBlockMarqueeSelection(editor, pointer("pointerdown", 80, 110), callbacks);
    window.dispatchEvent(pointer("pointermove", 450, 170));
    window.dispatchEvent(pointer("pointermove", 450, 170, 0));
    paint();
    expect(capture).toBe(true);
    expect(callbacks.onFinish).not.toHaveBeenCalled();
    expect(callbacks.onSelect).toHaveBeenLastCalledWith(["parent", "child"]);
    window.dispatchEvent(pointer("pointerup", 450, 200));
    expect(callbacks.onSelect).toHaveBeenLastCalledWith(["parent", "child", "peer"]);
    expect(callbacks.onFinish).toHaveBeenCalledExactlyOnceWith(false);
    expect(capture).toBe(false);
    expect(document.querySelector(".note-block-selection-rect")).toBeNull();
    window.dispatchEvent(pointer("pointerup", 450, 200));
    expect(callbacks.onFinish).toHaveBeenCalledTimes(1);
  });
  it("adjusts the fixed start point as the editor pane scrolls", () => {
    cleanup = startBlockMarqueeSelection(editor, pointer("pointerdown", 80, 170), callbacks);
    pane.scrollTop = 60;
    window.dispatchEvent(pointer("pointermove", 450, 200));
    paint();
    expect(callbacks.onSelect).toHaveBeenLastCalledWith(["parent", "child", "peer"]);
    expect(document.querySelector<HTMLElement>(".note-block-selection-rect")!.style.top).toBe("110px");
  });
  it("uses the editor edge velocity in one shared frame chain and idles away from the edge", () => {
    cleanup = startBlockMarqueeSelection(editor, pointer("pointerdown", 80, 300), callbacks);
    window.dispatchEvent(pointer("pointermove", 450, 490));
    expect(window.requestAnimationFrame).toHaveBeenCalledTimes(1);
    paint();
    expect(pane.scrollTop).toBeCloseTo(16 * 38 / 48);
    expect(window.requestAnimationFrame).toHaveBeenCalledTimes(2);
    window.dispatchEvent(pointer("pointermove", 450, 200)); paint();
    expect(window.requestAnimationFrame).toHaveBeenCalledTimes(2);
    expect(frame).toBeUndefined();
  });
  it.each(["escape", "pointercancel", "blur", "unmount", "capture-loss"])("cancels and removes feedback on %s", (reason) => {
    cleanup = startBlockMarqueeSelection(editor, pointer("pointerdown", 80, 110), callbacks);
    window.dispatchEvent(pointer("pointermove", 450, 170));
    if (reason === "escape") window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    if (reason === "pointercancel") window.dispatchEvent(pointer("pointercancel", 450, 170));
    if (reason === "blur") window.dispatchEvent(new Event("blur"));
    if (reason === "unmount") cleanup();
    if (reason === "capture-loss") editor.dispatchEvent(new Event("lostpointercapture"));
    window.dispatchEvent(pointer("pointerup", 450, 200));
    expect(callbacks.onFinish).toHaveBeenCalledExactlyOnceWith(true);
    expect(document.querySelector(".note-block-selection-rect")).toBeNull();
    expect(capture).toBe(false);
  });
  it("ignores other pointers and accepts coalesced zero-mask down/up input", () => {
    cleanup = startBlockMarqueeSelection(editor, pointer("pointerdown", 80, 110, 0), callbacks);
    window.dispatchEvent(pointer("pointermove", 450, 170, 1, 2));
    window.dispatchEvent(pointer("pointerup", 450, 200, 0, 2));
    expect(callbacks.onActivate).not.toHaveBeenCalled();
    window.dispatchEvent(pointer("pointerup", 450, 200));
    expect(callbacks.onSelect).toHaveBeenCalledExactlyOnceWith(["parent", "child", "peer"]);
    expect(callbacks.onFinish).toHaveBeenCalledExactlyOnceWith(false);
  });
});
