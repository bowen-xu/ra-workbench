import { useEffect, useRef, useState, type KeyboardEvent, type PointerEvent } from "react";
import { getAppStorage } from "../../../public/storage";

type Dock = "center" | "right";
const STORAGE_KEY = "research-assistant.writing-agent.panel-dock";
const PANEL_MARGIN = 18;
const SWITCH_FRACTION = 0.55;
const ATTRACTION_DURATION = 220;
interface Drag {
  readonly pointerId: number;
  readonly startX: number;
  readonly origin: number;
  readonly anchor: number;
  side: Dock;
  position: number;
  attraction: number;
  attractionTarget: number;
  readonly center: number;
  readonly right: number;
  readonly handle: HTMLButtonElement;
}
function readDock(): Dock {
  try { return getAppStorage().getItem(STORAGE_KEY) === "right" ? "right" : "center"; }
  catch { return "center"; }
}

/** Resist toward the pointer's side, then settle into that side's bottom anchor. */
export function useWritingAgentDock(open: boolean) {
  const panelRef = useRef<HTMLElement>(null);
  const dragRef = useRef<Drag | null>(null);
  const animationRef = useRef<number | null>(null);
  const [dock, setDock] = useState<Dock>(readDock);
  const [dragOffset, setDragOffset] = useState(0);
  const [dragging, setDragging] = useState(false);
  function stopAnimation(): void {
    if (animationRef.current !== null) window.cancelAnimationFrame(animationRef.current);
    animationRef.current = null;
  }
  function cancelDrag(): void {
    stopAnimation();
    const drag = dragRef.current;
    dragRef.current = null;
    setDragging(false); setDragOffset(0);
    if (drag?.handle.hasPointerCapture?.(drag.pointerId)) drag.handle.releasePointerCapture(drag.pointerId);
  }
  useEffect(() => {
    try { getAppStorage().setItem(STORAGE_KEY, dock); }
    catch { /* Keep the layout usable when local preferences cannot be saved. */ }
  }, [dock]);
  useEffect(() => {
    if (!open) cancelDrag();
  }, [open]);
  useEffect(() => () => stopAnimation(), []);

  function renderDrag(drag: Drag): void {
    setDragOffset((drag.attraction + drag.position) / 2 - drag.anchor);
  }
  function animateAttraction(drag: Drag, target: number): void {
    if (drag.attractionTarget === target) return;
    stopAnimation();
    drag.attractionTarget = target;
    if (Math.abs(drag.attraction - target) < 0.5 || window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
      drag.attraction = target;
      return;
    }
    const origin = drag.attraction, start = performance.now();
    const frame = (timestamp: number): void => {
      animationRef.current = null;
      if (dragRef.current !== drag) return;
      const progress = Math.min(1, Math.max(0, (timestamp - start) / ATTRACTION_DURATION));
      drag.attraction = origin + (target - origin) * (1 - (1 - progress) ** 3);
      renderDrag(drag);
      if (progress < 1) animationRef.current = window.requestAnimationFrame(frame);
    };
    animationRef.current = window.requestAnimationFrame(frame);
  }

  function pointerCenter(drag: Drag, clientX: number): number {
    // Preserve the grab offset: the header grip is not the panel's center.
    return Math.min(drag.right, Math.max(drag.center, drag.origin + clientX - drag.startX));
  }
  function targetDock(drag: Drag, position: number): Dock {
    const distance = drag.right - drag.center;
    // A dead band retains the attraction side even when the pointer jitters.
    if (drag.side === "center" && position > drag.center + distance * SWITCH_FRACTION) drag.side = "right";
    else if (drag.side === "right" && position < drag.right - distance * SWITCH_FRACTION) drag.side = "center";
    return drag.side;
  }
  function finish(event: PointerEvent<HTMLButtonElement>, canceled: boolean): void {
    const drag = dragRef.current;
    if (!drag || drag.pointerId !== event.pointerId) return;
    stopAnimation();
    dragRef.current = null;
    if (!canceled && Math.abs(event.clientX - drag.startX) >= 4 && drag.right > drag.center) {
      setDock(targetDock(drag, pointerCenter(drag, event.clientX)));
    }
    setDragging(false); setDragOffset(0);
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId);
  }
  const handleProps = {
    onPointerDown(event: PointerEvent<HTMLButtonElement>) {
      if (event.button !== 0 || dragRef.current) return;
      const panel = panelRef.current;
      if (!panel) return;
      const rect = panel.getBoundingClientRect();
      const container = panel.offsetParent?.getBoundingClientRect();
      const width = container?.width ?? window.innerWidth;
      const left = container?.left ?? 0;
      const center = left + width / 2;
      const right = Math.max(center, left + width - PANEL_MARGIN - rect.width / 2);
      const origin = rect.left + rect.width / 2;
      const anchor = dock === "right" ? right : center;
      dragRef.current = { pointerId: event.pointerId, startX: event.clientX, origin,
        anchor, side: dock, position: origin, attraction: origin, attractionTarget: origin,
        center, right, handle: event.currentTarget };
      event.preventDefault(); event.stopPropagation();
      event.currentTarget.setPointerCapture?.(event.pointerId);
      // A new drag may start while the preceding snap animation is in flight.
      setDragOffset(origin - anchor);
      setDragging(true);
    },
    onPointerMove(event: PointerEvent<HTMLButtonElement>) {
      const drag = dragRef.current;
      if (!drag || drag.pointerId !== event.pointerId) return;
      const position = pointerCenter(drag, event.clientX);
      const attraction = targetDock(drag, position) === "right" ? drag.right : drag.center;
      drag.position = position;
      // Only animate the attraction anchor, never delay the pointer contribution
      // or restart an in-flight side switch on ordinary pointer movements.
      animateAttraction(drag, attraction);
      renderDrag(drag);
    },
    onPointerUp: (event: PointerEvent<HTMLButtonElement>) => finish(event, false),
    onPointerCancel: (event: PointerEvent<HTMLButtonElement>) => finish(event, true),
    onLostPointerCapture: (event: PointerEvent<HTMLButtonElement>) => finish(event, true),
    onKeyDown(event: KeyboardEvent<HTMLButtonElement>) {
      if (event.key !== "ArrowLeft" && event.key !== "ArrowRight") return;
      event.preventDefault(); event.stopPropagation();
      cancelDrag();
      setDock(event.key === "ArrowLeft" ? "center" : "right");
    },
  };
  return { panelRef, dock, dragging, dragOffset, handleProps, cancelDrag };
}
