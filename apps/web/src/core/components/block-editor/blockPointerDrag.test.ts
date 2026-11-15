// @vitest-environment happy-dom
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { startBlockPointerDrag } from "./blockPointerDrag";

describe("internal BlockNote handle gesture", () => {
  let source: HTMLButtonElement;
  let editor: HTMLDivElement;
  let cancel: (() => void) | undefined;
  let captured: boolean;
  let hit: Element | null;
  const callbacks = { canDrop: vi.fn(() => true), move: vi.fn(), onActivate: vi.fn(), onFinish: vi.fn() };
  const pointer = (type: string, x: number, y: number, pointerId = 1, buttons = type === "pointerup" ? 0 : 1) => new PointerEvent(type, {
    bubbles: true, cancelable: true, pointerId, clientX: x, clientY: y, buttons,
  });
  beforeEach(() => {
    document.body.innerHTML = '<div class="block-editor"><button></button><div class="bn-editor"><div class="bn-block-outer"><div class="bn-block" data-id="target"><div class="bn-block-content"><p>target</p></div></div></div></div></div>';
    source = document.querySelector("button")!;
    editor = document.querySelector<HTMLDivElement>(".bn-editor")!;
    hit = editor.querySelector("p");
    captured = false;
    source.setPointerCapture = () => { captured = true; };
    source.hasPointerCapture = () => captured;
    source.releasePointerCapture = () => { captured = false; };
    vi.spyOn(editor, "getBoundingClientRect").mockReturnValue(new DOMRect(100, 100, 500, 400));
    vi.spyOn(editor.querySelector(".bn-block-outer")!, "getBoundingClientRect").mockReturnValue(new DOMRect(100, 180, 500, 80));
    vi.spyOn(editor.querySelector(".bn-block-content")!, "getBoundingClientRect").mockReturnValue(new DOMRect(100, 180, 500, 80));
    vi.spyOn(document, "elementFromPoint").mockImplementation(() => hit);
    vi.clearAllMocks();
    callbacks.canDrop.mockReturnValue(true);
    cancel = startBlockPointerDrag(source, editor, pointer("pointerdown", 80, 120), callbacks);
  });
  afterEach(() => { cancel?.(); vi.restoreAllMocks(); document.body.innerHTML = ""; });

  it("keeps clicks below threshold out of the drag pipeline", () => {
    window.dispatchEvent(pointer("pointermove", 82, 121));
    window.dispatchEvent(pointer("pointerup", 82, 121));
    expect(callbacks.onActivate).not.toHaveBeenCalled();
    expect(callbacks.move).not.toHaveBeenCalled();
  });
  it("keeps a released handle attached until its menu click is processed", () => {
    source.dispatchEvent(pointer("pointerup", 82, 121));
    expect(callbacks.onFinish).toHaveBeenCalledExactlyOnceWith({click: true});
    expect(callbacks.onActivate).not.toHaveBeenCalled();
    expect(callbacks.move).not.toHaveBeenCalled();
  });
  it("does not reserve a menu click after pointer cancellation", () => {
    source.dispatchEvent(pointer("pointercancel", 82, 121));
    expect(callbacks.onFinish).toHaveBeenCalledExactlyOnceWith({click: false});
  });
  it("moves exactly once by the displayed identity, without HTML5 drag events", () => {
    const htmlDrag = vi.fn();
    source.addEventListener("dragstart", htmlDrag);
    editor.addEventListener("drop", htmlDrag);
    window.dispatchEvent(pointer("pointermove", 450, 184));
    expect(captured).toBe(true);
    expect(document.querySelector(".note-block-drop-cursor")?.getAttribute("style")).toContain("top: 178.5px");
    window.dispatchEvent(pointer("pointerup", 450, 184));
    expect(callbacks.move).toHaveBeenCalledExactlyOnceWith({
      blockId: "target", placement: "before", line: { left: 100, top: 180, width: 500 },
    });
    expect(htmlDrag).not.toHaveBeenCalled();
    expect(callbacks.onFinish).toHaveBeenCalledTimes(1);
    expect(captured).toBe(false);
    expect(document.querySelector(".note-block-pointer-preview")).toBeNull();
    expect(document.querySelector(".note-block-drop-cursor")).toBeNull();
    expect(document.querySelector(".note-block-drop-target")).toBeNull();
    window.dispatchEvent(pointer("pointerup", 180, 220));
    expect(callbacks.move).toHaveBeenCalledTimes(1);
  });
  it("does not advertise or commit a rejected/self/no-op target", () => {
    callbacks.canDrop.mockReturnValue(false);
    window.dispatchEvent(pointer("pointermove", 160, 200));
    expect(document.querySelector<HTMLElement>(".note-block-drop-cursor")?.hidden).toBe(true);
    expect(document.querySelector<HTMLElement>(".note-block-drop-target")?.hidden).toBe(true);
    window.dispatchEvent(pointer("pointerup", 160, 200));
    expect(callbacks.move).not.toHaveBeenCalled();
  });
  it("highlights the entire accepted inside target, then clears it at a sibling boundary", () => {
    cancel?.();
    cancel = startBlockPointerDrag(source, editor, pointer("pointerdown", 80, 120), { ...callbacks, blockCount: 2 });
    window.dispatchEvent(pointer("pointermove", 160, 220));
    const target = document.querySelector<HTMLElement>(".note-block-drop-target")!;
    expect(target.hidden).toBe(false);
    expect(target.dataset.targetId).toBe("target");
    expect(target.style.top).toBe("180px");
    expect(target.style.height).toBe("80px");
    expect(document.querySelector(".note-block-pointer-preview")?.textContent).toBe("放入子块 · 2 个块");
    expect(document.querySelector<HTMLElement>(".note-block-drop-cursor")?.style.left).toBe("124px");
    window.dispatchEvent(pointer("pointermove", 160, 184));
    expect(target.hidden).toBe(true);
    expect(target.dataset.targetId).toBeUndefined();
    expect(document.querySelector(".note-block-pointer-preview")?.textContent).toBe("移动 2 个块");
    window.dispatchEvent(pointer("pointerup", 160, 220));
    expect(callbacks.move).toHaveBeenCalledExactlyOnceWith({
      blockId: "target", placement: "inside", line: { left: 124, top: 260, width: 476 },
      targetRect: { left: 100, top: 180, width: 500, height: 80 },
    });
    expect(document.querySelector(".note-block-drop-target")).toBeNull();
  });
  it.each([false, true])("waits for pointerup after a zero-buttons move (buttons recover: %s)", (recovers) => {
    window.dispatchEvent(pointer("pointermove", 80, 184));
    // Reproduce the user's captured 1 → 1 → 0 move sequence in the handle gutter.
    window.dispatchEvent(pointer("pointermove", 79.75, 184));
    window.dispatchEvent(pointer("pointermove", 79.5, 184, 1, 0));
    expect(captured).toBe(true);
    expect(callbacks.onFinish).not.toHaveBeenCalled();
    expect(callbacks.move).not.toHaveBeenCalled();
    expect(document.querySelector<HTMLElement>(".note-block-drop-cursor")?.hidden).toBe(false);
    if (recovers) window.dispatchEvent(pointer("pointermove", 80, 185));
    // Release in a different half: commit the release target, not a stale last target.
    window.dispatchEvent(pointer("pointerup", 80, 250));
    expect(callbacks.move).toHaveBeenCalledExactlyOnceWith({
      blockId: "target", placement: "after", line: { left: 100, top: 260, width: 500 },
    });
    expect(callbacks.onActivate).toHaveBeenCalledTimes(1);
    expect(callbacks.onFinish).toHaveBeenCalledTimes(1);
    expect(captured).toBe(false);
    expect(document.querySelector(".note-block-drop-cursor")).toBeNull();
    window.dispatchEvent(pointer("pointerup", 80, 250));
    expect(callbacks.move).toHaveBeenCalledTimes(1);
  });
  it("does not commit a previously valid target after zero-buttons movement and an outside release", () => {
    window.dispatchEvent(pointer("pointermove", 80, 184));
    window.dispatchEvent(pointer("pointermove", 80, 184, 1, 0));
    expect(callbacks.onFinish).not.toHaveBeenCalled();
    hit = document.body;
    window.dispatchEvent(pointer("pointerup", 20, 50));
    expect(callbacks.move).not.toHaveBeenCalled();
    expect(callbacks.onFinish).toHaveBeenCalledTimes(1);
  });
  it("does not drop into another workspace", () => {
    window.dispatchEvent(pointer("pointermove", 160, 200));
    hit = document.body;
    window.dispatchEvent(pointer("pointerup", 160, 200));
    expect(callbacks.move).not.toHaveBeenCalled();
  });
  it.each(["escape", "pointercancel", "blur", "unmount", "capture-loss"].flatMap((reason) => [
    { reason, zeroButtonsMove: false }, { reason, zeroButtonsMove: true },
  ]))("cleans up on $reason without a move (zero-buttons move: $zeroButtonsMove)", ({ reason, zeroButtonsMove }) => {
    window.dispatchEvent(pointer("pointermove", 160, 200));
    if (zeroButtonsMove) window.dispatchEvent(pointer("pointermove", 160, 200, 1, 0));
    expect(callbacks.onFinish).not.toHaveBeenCalled();
    if (reason === "escape") window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    if (reason === "pointercancel") window.dispatchEvent(pointer("pointercancel", 160, 200));
    if (reason === "blur") window.dispatchEvent(new Event("blur"));
    if (reason === "unmount") cancel?.();
    if (reason === "capture-loss") source.dispatchEvent(new Event("lostpointercapture"));
    window.dispatchEvent(pointer("pointerup", 180, 220));
    expect(callbacks.move).not.toHaveBeenCalled();
    expect(callbacks.onFinish).toHaveBeenCalledTimes(1);
    expect(captured).toBe(false);
    expect(document.querySelector(".note-block-pointer-preview")).toBeNull();
    expect(document.querySelector(".note-block-drop-cursor")).toBeNull();
  });
  it("ignores a second pointer", () => {
    window.dispatchEvent(pointer("pointermove", 160, 200, 2));
    window.dispatchEvent(pointer("pointerup", 180, 220, 2));
    expect(callbacks.onActivate).not.toHaveBeenCalled();
  });
  it("accepts a coalesced native down/up gesture with a missing buttons flag", () => {
    cancel?.();
    cancel = startBlockPointerDrag(source, editor, new PointerEvent("pointerdown", {
      pointerId: 1, clientX: 80, clientY: 120, buttons: 0,
    }), callbacks);
    window.dispatchEvent(new PointerEvent("pointerup", { pointerId: 1, clientX: 180, clientY: 220, buttons: 0 }));
    expect(callbacks.move).toHaveBeenCalledTimes(1);
  });
});
