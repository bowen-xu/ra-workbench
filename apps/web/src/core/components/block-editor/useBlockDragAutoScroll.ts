import { useCallback, useEffect, useRef } from "react";

const EDGE_SIZE = 48;
const MAX_SPEED = 16;

export function blockDragScrollVelocity(pointerY: number, top: number, bottom: number): number {
  if (pointerY < top + EDGE_SIZE) return -MAX_SPEED * Math.min(1, (top + EDGE_SIZE - pointerY) / EDGE_SIZE);
  if (pointerY > bottom - EDGE_SIZE) return MAX_SPEED * Math.min(1, (pointerY - (bottom - EDGE_SIZE)) / EDGE_SIZE);
  return 0;
}

export function useBlockDragAutoScroll(): {
  readonly start: (container: HTMLElement) => void;
  readonly stop: () => void;
} {
  const containerRef = useRef<HTMLElement | undefined>(undefined);
  const velocityRef = useRef(0);
  const frameRef = useRef<number | undefined>(undefined);
  const stopRef = useRef<() => void>(() => undefined);

  const stop = useCallback(() => {
    containerRef.current = undefined;
    velocityRef.current = 0;
    if (frameRef.current !== undefined) cancelAnimationFrame(frameRef.current);
    frameRef.current = undefined;
  }, []);

  const tick = useCallback(function run(): void {
    const container = containerRef.current;
    if (container === undefined) return;
    if (velocityRef.current !== 0) container.scrollTop += velocityRef.current;
    frameRef.current = requestAnimationFrame(run);
  }, []);

  const start = useCallback((container: HTMLElement) => {
    stopRef.current();
    containerRef.current = container;
    const handlePointerMove = (event: PointerEvent) => {
      const active = containerRef.current;
      if (active === undefined) return;
      const rect = active.getBoundingClientRect();
      velocityRef.current = blockDragScrollVelocity(event.clientY, rect.top, rect.bottom);
    };
    const handleStop = () => stopRef.current();
    // Use the real pointer, not the snapped block-boundary drag coordinates.
    window.addEventListener("pointermove", handlePointerMove);
    window.addEventListener("drop", handleStop, { once: true });
    window.addEventListener("dragend", handleStop, { once: true });
    frameRef.current = requestAnimationFrame(tick);
    const originalStop = stop;
    stopRef.current = () => {
      window.removeEventListener("pointermove", handlePointerMove);
      window.removeEventListener("drop", handleStop);
      window.removeEventListener("dragend", handleStop);
      originalStop();
    };
  }, [stop, tick]);

  useEffect(() => () => stopRef.current(), []);
  return { start, stop: () => stopRef.current() };
}
