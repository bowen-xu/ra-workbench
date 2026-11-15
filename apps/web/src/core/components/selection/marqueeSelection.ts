import { mountOverlayFeedback } from "../overlay/overlayScope";
import "./rowSelection.css";
export interface MarqueePoint { readonly clientX: number; readonly clientY: number }
export interface MarqueeCallbacks {
  readonly scrollContainer?: HTMLElement;
  readonly className?: string;
  readonly autoScroll?: boolean;
  readonly scrollVelocity?: (pointer: MarqueePoint, bounds: DOMRect) => number;
  readonly getIds: (start: MarqueePoint, end: MarqueePoint) => readonly string[];
  readonly onActivate: () => void;
  readonly onSelect: (ids: readonly string[]) => void;
  readonly onFinish: (cancelled: boolean, activated: boolean) => void;
}
type Point = MarqueePoint;

/** Independent block-selection gesture, not a ProseMirror text selection. */
export function startMarqueeSelection(editor: HTMLElement, initial: PointerEvent, callbacks: MarqueeCallbacks): () => void {
  const document = editor.ownerDocument;
  const window = document.defaultView!;
  const pane = callbacks.scrollContainer ?? editor;
  const initialScroll = pane.scrollTop;
  let scrollTop = initialScroll;
  let pointer: Point = initial;
  let active = false;
  let finished = false;
  let box: HTMLElement | undefined;
  let disposeOverlay: (() => void) | undefined;
  let frame: number | undefined;
  let dirty = false;
  let previousIds: readonly string[] | undefined;
  let previousResult: readonly string[] | undefined;

  function update(bounds: DOMRect): void {
    const start = { clientX: initial.clientX, clientY: initial.clientY - (pane.scrollTop - initialScroll) };
    dirty = false;
    const ids = callbacks.getIds(start, pointer);
    if (ids !== previousResult && (previousIds === undefined || ids.length !== previousIds.length || ids.some((id, index) => id !== previousIds![index]))) {
      previousIds = [...ids];
      callbacks.onSelect(ids);
    }
    previousResult = ids;
    if (box === undefined) return;
    const left = Math.max(bounds.left, Math.min(start.clientX, pointer.clientX));
    const right = Math.min(bounds.right, Math.max(start.clientX, pointer.clientX));
    const top = Math.max(bounds.top, Math.min(start.clientY, pointer.clientY));
    const bottom = Math.min(bounds.bottom, Math.max(start.clientY, pointer.clientY));
    Object.assign(box.style, { left: `${left}px`, top: `${top}px`, width: `${Math.max(0, right - left)}px`, height: `${Math.max(0, bottom - top)}px` });
  }
  function trackScroll(): void {
    frame = undefined;
    if (finished) return;
    const bounds = pane.getBoundingClientRect();
    const beforeScroll = pane.scrollTop;
    if (callbacks.autoScroll) {
      const delta = callbacks.scrollVelocity?.(pointer, bounds)
        ?? (pointer.clientY < bounds.top + 32 ? -12 : pointer.clientY > bounds.bottom - 32 ? 12 : 0);
      if (delta) pane.scrollTop += delta;
    }
    if (scrollTop !== pane.scrollTop) { scrollTop = pane.scrollTop; dirty = true; }
    if (dirty) update(bounds);
    // Only an edge that actually scrolls needs another frame. A stationary
    // pointer in the interior (or at a scroll limit) leaves no idle loop.
    if (pane.scrollTop !== beforeScroll) schedule();
  }
  function schedule(): void {
    if (active && !finished && frame === undefined) frame = window.requestAnimationFrame(trackScroll);
  }
  function scroll(): void {
    if (pane.scrollTop === scrollTop) return;
    dirty = true; schedule();
  }
  function resize(): void {
    dirty = true; schedule();
  }
  function finish(cancelled: boolean): void {
    if (finished) return;
    finished = true;
    window.removeEventListener("pointermove", move, true);
    window.removeEventListener("pointerup", up, true);
    window.removeEventListener("pointercancel", cancel, true);
    window.removeEventListener("keydown", keyDown, true);
    window.removeEventListener("blur", abort);
    pane.removeEventListener("scroll", scroll);
    window.removeEventListener("resize", resize);
    editor.removeEventListener("lostpointercapture", abort);
    if (frame !== undefined) window.cancelAnimationFrame(frame);
    box?.remove();
    disposeOverlay?.();
    if (editor.hasPointerCapture(initial.pointerId)) editor.releasePointerCapture(initial.pointerId);
    callbacks.onFinish(cancelled, active);
  }
  function activate(capture: boolean): void {
    if (active || Math.hypot(pointer.clientX - initial.clientX, pointer.clientY - initial.clientY) < 6) return;
    active = true;
    callbacks.onActivate();
    document.getSelection()?.removeAllRanges();
    if (capture && initial.buttons & 1) editor.setPointerCapture(initial.pointerId);
    box = document.createElement("div");
    box.className = callbacks.className ?? "row-selection-marquee";
    box.setAttribute("aria-hidden", "true");
    disposeOverlay = mountOverlayFeedback(editor, box);
  }
  function move(event: PointerEvent): void {
    if (event.pointerId !== initial.pointerId) return;
    const moved = event.clientX !== pointer.clientX || event.clientY !== pointer.clientY;
    pointer = event;
    activate(true);
    if (!active || finished) return;
    event.preventDefault();
    if (!moved) return;
    // Pointer devices may deliver several moves per paint. Only the latest
    // rectangle matters; the existing scroll frame also flushes pointer work.
    dirty = true;
    schedule();
  }
  function up(event: PointerEvent): void {
    if (event.pointerId !== initial.pointerId) return;
    pointer = event;
    try {
      activate(false);
      if (active && !finished) { event.preventDefault(); update(pane.getBoundingClientRect()); }
    } finally { finish(false); }
  }
  function abort(): void { finish(true); }
  function cancel(event: PointerEvent): void { if (event.pointerId === initial.pointerId) abort(); }
  function keyDown(event: KeyboardEvent): void {
    if (event.key !== "Escape") return;
    event.preventDefault();
    event.stopPropagation();
    abort();
  }
  window.addEventListener("pointermove", move, true);
  window.addEventListener("pointerup", up, true);
  window.addEventListener("pointercancel", cancel, true);
  window.addEventListener("keydown", keyDown, true);
  window.addEventListener("blur", abort);
  pane.addEventListener("scroll", scroll, {passive: true});
  window.addEventListener("resize", resize);
  editor.addEventListener("lostpointercapture", abort);
  return abort;
}
