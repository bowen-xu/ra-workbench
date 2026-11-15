interface MathNode {
  type: string;
  tagName?: string;
  properties?: Record<string, unknown>;
  children?: MathNode[];
}

/** Wrap a complete rendered formula, preserving KaTeX's MathML and script sizing. */
export function rehypeMathViewport() {
  return (tree: MathNode): void => {
    function visit(parent: MathNode): void {
      if (!parent.children) return;
      parent.children = parent.children.map(node => {
        const classes = node.properties?.className;
        const display = Array.isArray(classes) && classes.includes("katex-display");
        const inline = Array.isArray(classes) && classes.includes("katex");
        if (display || inline) return {
          type: "element", tagName: "agent-math",
          properties: { className: ["agent-math-viewport"], "data-math-display": display },
          children: [node],
        };
        visit(node);
        return node;
      });
    }
    visit(tree);
  };
}
