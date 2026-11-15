import {blockNoteDragDom, type BlockDragDom} from "./blockDragDom";
import { captureBlockDropGeometry, resolveBlockDropTarget, type BlockDropTarget } from "./blockDropTarget";
import { getEditorScrollContainer } from "./editorSurface";
import { createBlockReorderPreview } from "./blockReorderPreview";
import { mountOverlayFeedback } from "../overlay/overlayScope";

interface BlockDragCallbacks {
  readonly blockCount?: number;
  readonly sourceIds?: readonly string[];
  readonly canDrop: (target: BlockDropTarget) => boolean;
  readonly move: (target: BlockDropTarget) => void;
  readonly onActivate: () => void;
  readonly onFinish: (result: {readonly click: boolean}) => void;
}

/** Pointer-only block gesture. No native drag session, clipboard slice or caret
 * selection participates in the move; the editor command owns document changes.
 */
export function startBlockPointerDrag(
  source: HTMLElement,
  editor: HTMLElement,
  initial: PointerEvent,
  callbacks: BlockDragCallbacks,
  dom: BlockDragDom = blockNoteDragDom,
): () => void {
  const document = source.ownerDocument;
  const window = document.defaultView!;
  const scrollContainer = getEditorScrollContainer(editor);
  const geometry = captureBlockDropGeometry(editor, dom);
  const reorder = callbacks.sourceIds ? createBlockReorderPreview(editor, callbacks.sourceIds, dom) : undefined;
  let pointer = initial;
  let active = false;
  let finished = false;
  let click = false;
  let preview: HTMLElement | undefined;
  let cursor: HTMLElement | undefined;
  let targetHighlight: HTMLElement | undefined;
  let disposeOverlay: (() => void) | undefined;
  let frame: number | undefined;
  let scrollTop = scrollContainer?.scrollTop;

  function updateTarget(): BlockDropTarget | undefined {
    const target = resolveBlockDropTarget(editor, pointer, geometry, dom);
    const accepted = target !== undefined && callbacks.canDrop(target) ? target : undefined;
    const motion = reorder?.update(accepted);
    if (preview !== undefined) {
      preview.style.transform = `translate(${pointer.clientX + 14}px, ${pointer.clientY + 14}px)`;
      preview.textContent = accepted?.placement === "inside" ? `放入子块 · ${callbacks.blockCount ?? 1} 个块`
        : (callbacks.blockCount ?? 1) > 1 ? `移动 ${callbacks.blockCount} 个块` : "移动块";
    }
    if (targetHighlight !== undefined) {
      targetHighlight.hidden = accepted?.targetRect === undefined;
      if (accepted?.targetRect !== undefined) {
        const rect = accepted.targetRect;
        const clip = scrollContainer?.getBoundingClientRect();
        const shiftedTop = rect.top + (motion?.shift ?? 0);
        const shiftedBottom = shiftedTop + rect.height + (motion?.growth ?? 0);
        const top = Math.max(shiftedTop, clip?.top ?? shiftedTop);
        const bottom = Math.min(shiftedBottom, clip?.bottom ?? shiftedBottom);
        Object.assign(targetHighlight.style, { left: `${rect.left}px`, top: `${top}px`, width: `${rect.width}px`, height: `${Math.max(0, bottom - top)}px` });
        targetHighlight.dataset.targetId = accepted.blockId;
      } else delete targetHighlight.dataset.targetId;
    }
    if (cursor !== undefined) {
      cursor.hidden = accepted === undefined;
      if (accepted !== undefined) {
        cursor.style.left = `${accepted.line.left}px`;
        cursor.style.top = `${accepted.line.top + (motion?.shift ?? 0) + (accepted.placement === "before" ? 0 : motion?.growth ?? 0) - 1.5}px`;
        cursor.style.width = `${accepted.line.width}px`;
      }
    }
    return accepted;
  }

  function trackScroll(): void {
    if (finished) return;
    if (scrollTop !== scrollContainer?.scrollTop) {
      scrollTop = scrollContainer?.scrollTop;
      updateTarget();
    }
    frame = window.requestAnimationFrame(trackScroll);
  }

  function finish(): void {
    if (finished) return;
    finished = true;
    window.removeEventListener("pointermove", move, true);
    window.removeEventListener("pointerup", drop, true);
    window.removeEventListener("pointercancel", cancel, true);
    window.removeEventListener("keydown", keydown, true);
    window.removeEventListener("blur", finish);
    source.removeEventListener("lostpointercapture", finish);
    if (frame !== undefined) window.cancelAnimationFrame(frame);
    preview?.remove();
    cursor?.remove();
    targetHighlight?.remove();
    disposeOverlay?.();
    reorder?.dispose();
    if (source.hasPointerCapture(initial.pointerId)) source.releasePointerCapture(initial.pointerId);
    callbacks.onFinish({click});
  }

  function activate(capture: boolean): void {
    if (active || Math.hypot(pointer.clientX - initial.clientX, pointer.clientY - initial.clientY) < 6) return;
    active = true;
    callbacks.onActivate();
    if (capture && initial.buttons & 1) source.setPointerCapture(initial.pointerId);
    preview = document.createElement("div");
    preview.className = "note-block-pointer-preview";
    preview.setAttribute("aria-hidden", "true");
    preview.textContent = "移动块";
    cursor = document.createElement("div");
    cursor.className = "note-block-drop-cursor";
    cursor.setAttribute("aria-hidden", "true");
    cursor.hidden = true;
    targetHighlight = document.createElement("div");
    targetHighlight.className = "note-block-drop-target";
    targetHighlight.setAttribute("aria-hidden", "true");
    targetHighlight.hidden = true;
    disposeOverlay = mountOverlayFeedback(editor, preview, cursor, targetHighlight);
    frame = window.requestAnimationFrame(trackScroll);
  }

  function move(event: PointerEvent): void {
    if (event.pointerId !== initial.pointerId) return;
    // Captured Tauri gestures can report buttons=0 on a move before pointerup.
    // Only explicit up/cancel/capture loss ends the gesture; ending here removes
    // the up listener and silently discards an accepted drop. See the regression
    // evidence in docs/testing/note-block-drag-regression.md.
    pointer = event;
    activate(true);
    if (!active || finished) return;
    event.preventDefault();
    updateTarget();
  }

  function drop(event: PointerEvent): void {
    if (event.pointerId !== initial.pointerId) return;
    pointer = event;
    try {
      // Accommodate native input that coalesces all intermediate moves.
      activate(false);
      click = !active && event.target instanceof Node && source.contains(event.target);
      if (active && !finished) {
        event.preventDefault();
        const target = updateTarget();
        if (target !== undefined) callbacks.move(target);
      }
    } finally { finish(); }
  }

  function cancel(event: PointerEvent): void { if (event.pointerId === initial.pointerId) finish(); }
  function keydown(event: KeyboardEvent): void {
    if (event.key !== "Escape") return;
    if (active) { event.preventDefault(); event.stopPropagation(); }
    finish();
  }
  window.addEventListener("pointermove", move, true);
  window.addEventListener("pointerup", drop, true);
  window.addEventListener("pointercancel", cancel, true);
  window.addEventListener("keydown", keydown, true);
  window.addEventListener("blur", finish);
  source.addEventListener("lostpointercapture", finish);
  return finish;
}
