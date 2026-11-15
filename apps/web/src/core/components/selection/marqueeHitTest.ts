import type { MarqueePoint } from "./marqueeSelection";

interface RowBounds {
  readonly id: string;
  readonly left: number;
  readonly right: number;
  readonly top: number;
  readonly bottom: number;
}

/** Gesture-local geometry in scroll coordinates. Selection paint does not require
 * another layout read; content, layout and ancestor scrolling invalidate it.
 */
export function createMarqueeHitTest(root: HTMLElement, pane: HTMLElement, selector: string, getId: (row: HTMLElement) => string | undefined): {
  readonly getIds: (start: MarqueePoint, end: MarqueePoint) => readonly string[];
  readonly dispose: () => void;
} {
  const window = root.ownerDocument.defaultView!;
  let rows: readonly RowBounds[] | undefined;
  const observed = new Set<Element>();
  let disposed = false;
  const invalidate = () => { rows = undefined; };
  const resize = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(invalidate);
  const mutations = new MutationObserver(invalidate);
  mutations.observe(root, {subtree: true, childList: true, characterData: true, attributes: true,
    attributeFilter: ["style", "class", "data-id", "data-selection-id", "data-content-type"]});
  // Scrolling this pane is just an offset; other scroll containers move the
  // cached viewport geometry and must be measured again.
  const scroll = (event: Event) => { if (event.target !== pane) invalidate(); };
  window.addEventListener("scroll", scroll, true);
  window.addEventListener("resize", invalidate);

  function measure(): readonly RowBounds[] {
    const elements = Array.from(root.querySelectorAll<HTMLElement>(selector));
    const targets = new Set<Element>([root, pane, ...elements]);
    for (const target of observed) if (!targets.has(target)) { resize?.unobserve(target); observed.delete(target); }
    for (const target of targets) if (!observed.has(target)) { resize?.observe(target); observed.add(target); }
    const scrollLeft = pane.scrollLeft, scrollTop = pane.scrollTop;
    return elements.flatMap(row => {
      const id = getId(row), bounds = row.getBoundingClientRect();
      return id !== undefined && bounds.width > 0 && bounds.height > 0 ? [{id,
        left: bounds.left + scrollLeft, right: bounds.right + scrollLeft,
        top: bounds.top + scrollTop, bottom: bounds.bottom + scrollTop}] : [];
    });
  }
  return {
    getIds(start, end) {
      if (disposed) return [];
      // A pointerup may precede observer delivery. Never commit stale geometry.
      if (mutations.takeRecords().length) invalidate();
      rows ??= measure();
      const left = Math.min(start.clientX, end.clientX) + pane.scrollLeft;
      const right = Math.max(start.clientX, end.clientX) + pane.scrollLeft;
      const top = Math.min(start.clientY, end.clientY) + pane.scrollTop;
      const bottom = Math.max(start.clientY, end.clientY) + pane.scrollTop;
      return rows.filter(row => row.left < right && row.right > left && row.top < bottom && row.bottom > top).map(row => row.id);
    },
    dispose() {
      disposed = true;
      rows = undefined;
      observed.clear();
      resize?.disconnect(); mutations.disconnect();
      window.removeEventListener("scroll", scroll, true);
      window.removeEventListener("resize", invalidate);
    },
  };
}
