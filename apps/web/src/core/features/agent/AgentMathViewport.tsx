import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import "./AgentMathViewport.css";

/** Only oversized formulas become scroll regions. Vertical size remains intrinsic. */
export function AgentMathViewport({ children, display }: {
  readonly children: ReactNode;
  readonly display: boolean;
}): React.JSX.Element {
  const viewport = useRef<HTMLSpanElement>(null);
  const [overflow, setOverflow] = useState(false);
  useLayoutEffect(() => {
    const element = viewport.current;
    if (!element) return;
    const formula = element.querySelector(".katex");
    let disposed = false;
    const measure = (): void => {
      // KaTeX scripts and glyph overhang can increase scrollWidth even for a
      // tiny inline formula. Comparing that to its shrink-wrapped clientWidth
      // toggles inline/block layout on every ResizeObserver callback. Measure
      // the complete intrinsic formula against the actual bounded viewport;
      // the comparison remains valid after entering/leaving the scroll region.
      if (!disposed && formula) setOverflow(formula.getBoundingClientRect().width > element.getBoundingClientRect().width + 1);
    };
    measure();
    const observer = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(measure);
    observer?.observe(element);
    if (formula) observer?.observe(formula);
    // Font loading can change an existing formula without a new model chunk.
    void element.ownerDocument.fonts?.ready.then(measure);
    return () => { disposed = true; observer?.disconnect(); };
  }, [children]);
  return <span ref={viewport} className="agent-math-viewport" data-math-display={display}
    data-overflow={overflow} tabIndex={overflow ? 0 : undefined}
    role={overflow ? "region" : undefined} aria-label={overflow ? "公式，可左右滚动" : undefined}>
    {children}
  </span>;
}
