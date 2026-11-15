interface RevealNode {
  type: string;
  tagName?: string;
  value?: string;
  properties?: Record<string, unknown>;
  children?: RevealNode[];
  position?: { start: { offset?: number | undefined }; end: { offset?: number | undefined } } | undefined;
}
interface RevealOptions { readonly enabled: boolean; readonly source: string; }
const graphemes = new Intl.Segmenter(undefined, { granularity: "grapheme" });
function isMath(node: RevealNode): boolean {
  const classes = node.properties?.className;
  return Array.isArray(classes) && classes.some(value => ["math-inline", "math-display", "language-math"].includes(value));
}
function wrapper(node: RevealNode, start: number, end: number, atomic = false): RevealNode {
  const block = node.tagName === "pre" || (Array.isArray(node.properties?.className) && node.properties.className.includes("math-display"));
  return { type: "element", tagName: "span", properties: {
    className: ["stream-reveal-glyph", ...(atomic ? [block ? "is-block" : "is-atomic"] : [])],
    "data-reveal-start": start, "data-reveal-end": Math.max(start + 1, end),
    style: "--stream-reveal-progress:0",
  }, children: [node] };
}

/** Annotate before KaTeX replaces the formula source with generated markup.
 * Text remains in the DOM once; wrappers preserve graphemes and Markdown semantics. */
export function rehypeStreamReveal({ enabled, source }: RevealOptions) {
  return (tree: RevealNode): void => {
    if (!enabled) return;
    function visit(parent: RevealNode): void {
      if (!parent.children) return;
      parent.children = parent.children.flatMap(node => {
        const start = node.position?.start.offset ?? parent.position?.start.offset ?? 0;
        const end = node.position?.end.offset ?? parent.position?.end.offset ?? source.length;
        // Keep a math code fence's pre/code relationship intact for rehype-katex.
        if (isMath(node) || (node.tagName === "pre" && node.children?.some(isMath))) {
          return [wrapper(node, start, end, true)];
        }
        if (node.type !== "text" || !node.value || /^\s*$/u.test(node.value)) {
          visit(node); return [node];
        }
        const text = node.value;
        const exactStart = source.indexOf(text, start);
        const sourceStart = exactStart >= start && exactStart + text.length <= end ? exactStart : start;
        const scale = sourceStart === exactStart ? 1 : (end - start) / text.length;
        return [...graphemes.segment(text)].map(({ segment, index }) => wrapper(
          { type: "text", value: segment }, sourceStart + index * scale, sourceStart + (index + segment.length) * scale));
      });
    }
    visit(tree);
  };
}
