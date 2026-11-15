import {fromDom} from "hast-util-from-dom";
import {toMdast} from "hast-util-to-mdast";
import {toMarkdown} from "mdast-util-to-markdown";
import {gfmToMarkdown} from "mdast-util-gfm";
import {formulaClipboardElement, formulaSelector, readFormula} from "./formulaClipboard";
import type {} from "mdast-util-math";

export interface RichClipboardFragment {readonly text: string; readonly html: string;}

function elementAt(node: Node): Element | null {return node instanceof Element ? node : node.parentElement;}

/** Clip to one owner and keep the selection's ancestor structure (including a
 * partial bold/link/list item). Formula selections expand to the whole atom. */
function selectedDOM(root: HTMLElement, selection: Range): HTMLElement {
  const document = root.ownerDocument, range = document.createRange();
  range.selectNodeContents(root);
  if (root.contains(selection.startContainer)) range.setStart(selection.startContainer, selection.startOffset);
  if (root.contains(selection.endContainer)) range.setEnd(selection.endContainer, selection.endOffset);
  const startMath = elementAt(range.startContainer)?.closest(formulaSelector);
  const endMath = elementAt(range.endContainer)?.closest(formulaSelector);
  if (startMath && root.contains(startMath)) range.setStartBefore(startMath);
  if (endMath && root.contains(endMath)) range.setEndAfter(endMath);
  let fragment: DocumentFragment | HTMLElement = range.cloneContents();
  let ancestor = elementAt(range.commonAncestorContainer);
  while (ancestor && ancestor !== root && root.contains(ancestor)) {
    const wrapper = ancestor.cloneNode(false) as HTMLElement;
    wrapper.append(fragment); fragment = wrapper; ancestor = ancestor.parentElement;
  }
  const container = document.createElement("div"); container.append(fragment);
  container.querySelectorAll("button, [data-agent-message-actions], script, style").forEach(node => node.remove());
  for (const candidate of container.querySelectorAll(`${formulaSelector}, .katex-display, .katex, math`)) {
    if (!container.contains(candidate)) continue;
    const formula = readFormula(candidate);
    if (formula) candidate.replaceWith(formulaClipboardElement(document, formula.source, formula.display, formula.math ?? undefined));
  }
  // Reveal wrappers and viewport styles are UI state, not clipboard formatting.
  for (const node of container.querySelectorAll(".stream-reveal-glyph, .writing-agent-code-block")) node.replaceWith(...node.childNodes);
  for (const node of container.querySelectorAll("*")) {
    const language = node.tagName === "CODE" ? [...node.classList].find(value => value.startsWith("language-")) : undefined;
    node.removeAttribute("class");
    if (language) node.setAttribute("class", language);
    node.removeAttribute("style"); node.removeAttribute("aria-hidden");
    node.removeAttribute("id"); node.removeAttribute("tabindex");
  }
  return container;
}

/** Convert only the selected semantic fragment. The standard HAST/MDAST
 * adapters own Markdown quoting, nesting, tables, links and code fences. */
export function selectionClipboard(root: HTMLElement, range: Range): RichClipboardFragment {
  const selected = selectedDOM(root, range), html = selected.innerHTML;
  const markdownDOM = selected.cloneNode(true) as HTMLElement;
  for (const node of markdownDOM.querySelectorAll('[data-source]')) {
    const formula = readFormula(node);
    if (!formula) continue;
    const marker = root.ownerDocument.createElement(formula.display ? "clipboard-block-math" : "span");
    marker.setAttribute("data-source", formula.source);
    // HTML whitespace normalization must see a nonempty inline atom, otherwise
    // it can collapse the text space following an empty custom marker.
    marker.textContent = formula.source || "\uFFFC";
    node.replaceWith(marker);
  }
  const tree = toMdast(fromDom(markdownDOM), {handlers: {
    "clipboard-block-math": (_state, node) => ({type: "math", value: String(node.properties.dataSource ?? "")}),
    span: (state, node) => node.properties.dataSource === undefined ? state.all(node)
      : {type: "inlineMath", value: String(node.properties.dataSource)},
  }});
  const text = toMarkdown(tree, {extensions: [gfmToMarkdown()], handlers: {
    math: node => `\\[\n${node.value}\n\\]`,
    inlineMath: node => `\\(${node.value}\\)`,
  }}).trimEnd();
  return {text, html};
}
