import {useLayoutEffect, useState, type RefObject} from "react";

/** Presentation hysteresis is independent of the transcript's follow policy.
 * Reusing this state for follow would pull readers back from near the end. */
export function useReturnToBottomVisibility(
  viewportRef: RefObject<HTMLDivElement | null>,
  contentRef: RefObject<HTMLDivElement | null>,
  open: boolean,
) {
  const [visible, setVisible] = useState(false);
  useLayoutEffect(() => {
    const viewport = viewportRef.current;
    if (!open || !viewport) {setVisible(false); return;}
    const measure = () => {
      const distance = Math.max(0, viewport.scrollHeight - viewport.clientHeight - viewport.scrollTop);
      // Hide before reaching the very bottom; retain state in the dead band.
      setVisible(previous => distance <= 25 ? false : distance >= 30 ? true : previous);
    };
    measure();
    viewport.addEventListener("scroll", measure, {passive: true});
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(measure);
    observer?.observe(viewport);
    if (contentRef.current) observer?.observe(contentRef.current);
    return () => {viewport.removeEventListener("scroll", measure); observer?.disconnect();};
  }, [viewportRef, contentRef, open]);
  return {visible, hide: () => setVisible(false)};
}
