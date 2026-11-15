import {useLayoutEffect, useRef, type RefObject} from "react";
import {createWheelGestureIntent} from "./wheelGestureIntent";

/** Scroll expanded content first, then turn at its edge on a new active gesture.
 * Consuming one page keeps its remaining packets (including inertia) on that page. */
export function useScrollPageNavigation(viewport: RefObject<HTMLElement | null>, turn: (direction: -1 | 1) => void): void {
  const latest = useRef(turn);
  useLayoutEffect(() => { latest.current = turn; }, [turn]);
  useLayoutEffect(() => {
    if (!viewport.current) return;
    const node = viewport.current;
    const intent = createWheelGestureIntent();
    let consumed = false, pageChanged = false, previousDirection = 0;
    function wheel(event: WheelEvent): void {
      if (event.ctrlKey || event.metaKey) return;
      if (event.deltaX === 0 && event.deltaY === 0) { intent.feed(event); return; }
      if (Math.abs(event.deltaX) >= Math.abs(event.deltaY)) return;
      const phase = intent.feed(event);
      const direction = event.deltaY > 0 ? 1 : -1;
      const reversed = previousDirection !== 0 && previousDirection !== direction;
      if (!phase.momentum && (phase.start || reversed)) { consumed = false; pageChanged = false; }
      previousDirection = direction;
      const canScroll = direction > 0 ? node.scrollHeight - node.clientHeight - node.scrollTop > 1 : node.scrollTop > 1;
      event.stopPropagation();
      if (!pageChanged && canScroll) { consumed = true; return; }
      if (event.cancelable) event.preventDefault();
      if (phase.momentum || consumed) return;
      consumed = true; pageChanged = true;
      latest.current(direction);
    }
    node.addEventListener("wheel", wheel, {passive: false});
    return () => { node.removeEventListener("wheel", wheel); intent.dispose(); };
  }, [viewport]);
}
