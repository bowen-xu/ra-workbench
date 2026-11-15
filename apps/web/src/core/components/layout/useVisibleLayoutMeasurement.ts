import { useLayoutEffect, useRef } from "react";

/** Hidden retained hosts have no layout box; that is not a new user layout. */
export function visibleLayoutRect(element: HTMLElement | null): DOMRect | undefined {
  if (!element) return undefined;
  const bounds = element.getBoundingClientRect();
  return bounds.width > 0 && bounds.height > 0 ? bounds : undefined;
}

/**
 * Keep valid measurements while hidden, and measure every committed reveal
 * before paint. ResizeObserver alone can leave the previous geometry on reveal.
 * The resolver also handles retained hosts whose measured DOM owner changes.
 */
export function useVisibleLayoutMeasurement<T extends HTMLElement>(
  target: () => T | null,
  measure: (element: T, bounds: DOMRect) => void,
): void {
  const current = useRef({ target, measure });
  const observed = useRef<{ element: T; observer: ResizeObserver | undefined } | undefined>(undefined);
  const run = useRef(() => {
    const element = current.current.target();
    const bounds = visibleLayoutRect(element);
    if (element && bounds) current.current.measure(element, bounds);
  }).current;

  useLayoutEffect(() => {
    current.current = { target, measure };
    const element = target();
    if (observed.current?.element !== element) {
      observed.current?.observer?.disconnect();
      observed.current = undefined;
      if (element) {
        const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(run);
        observer?.observe(element);
        observed.current = { element, observer };
      }
    }
    run();
  });

  useLayoutEffect(() => {
    let mounted = true;
    window.addEventListener("resize", run);
    void document.fonts?.ready.then(() => { if (mounted) run(); });
    return () => {
      mounted = false;
      window.removeEventListener("resize", run);
      observed.current?.observer?.disconnect();
      observed.current = undefined;
    };
  }, [run]);
}
