import type {Element, Root, RootContent} from "hast";
import {toHtml} from "hast-util-to-html";
import {markdownToHast} from "./markdownToHast";
import {sanitizeMarkdownHast} from "./sanitizeMarkdownHast";

const mathNamespace = "http://www.w3.org/1998/Math/MathML";
export const formulaSelector = '.agent-math-viewport, [data-inline-content-type="inlineFormula"], [data-content-type="formulaBlock"]';

/** Portable HTML: one Presentation MathML tree plus its TeX annotation/source.
 * Native editor markers are semantic import hints; no document/block IDs travel. */
export function formulaClipboardElement(document: Document, source: string, display: boolean, math?: globalThis.Element): HTMLElement {
  const element = document.createElement(display ? "div" : "span");
  element.setAttribute(display ? "data-content-type" : "data-inline-content-type", display ? "formulaBlock" : "inlineFormula");
  element.setAttribute("data-source", source);
  if (math) {
    const copy = math.cloneNode(true) as globalThis.Element;
    copy.setAttribute("xmlns", mathNamespace);
    copy.setAttribute("display", display ? "block" : "inline");
    element.append(copy);
  } else element.textContent = source;
  return element;
}

export function readFormula(element: globalThis.Element): {source: string; display: boolean; math: globalThis.Element | null} | null {
  const math = element.matches("math") ? element : element.querySelector("math");
  const source = element.getAttribute("data-source") ?? math?.querySelector('annotation[encoding="application/x-tex"]')?.textContent;
  if (source === undefined || source === null) return null;
  return {source, math, display: element.getAttribute("data-math-display") === "true"
    || element.getAttribute("data-content-type") === "formulaBlock"
    || element.matches(".katex-display") || math?.getAttribute("display") === "block"};
}

/** External HTML may retain only MathML/TeX, or KaTeX's two representations.
 * Consume each complete formula once before the native editor HTML parser. */
export function normalizeFormulaHTML(html: string, document: Document): string | null {
  const template = document.createElement("template"); template.innerHTML = html;
  let changed = false;
  const candidates = template.content.querySelectorAll(`${formulaSelector}, .katex-display, .katex, math`);
  for (const candidate of candidates) {
    if (!template.content.contains(candidate)) continue;
    const formula = readFormula(candidate);
    if (!formula) continue;
    candidate.replaceWith(formulaClipboardElement(document, formula.source, formula.display, formula.math ?? undefined));
    changed = true;
  }
  return changed ? template.innerHTML : null;
}

/** Only take the custom paste path if this grammar actually found math. Native
 * Markdown import remains the owner for documents without formulas. Raw HTML
 * inside Markdown stays literal, matching the chat renderer's safety boundary. */
export function markdownFormulaHTML(markdown: string): string | null {
  const tree = markdownToHast(markdown);
  let found = false;
  function visit(parent: Root | Element): void {
    parent.children = parent.children.map((node): RootContent => {
      if (node.type === "raw") return {type: "text", value: node.value};
      if (node.type !== "element") return node;
      const classes = node.properties.className;
      if (node.tagName === "code" && Array.isArray(classes) && classes.includes("language-math")) {
        found = true;
        const display = classes.includes("math-display");
        const source = node.children.map(child => child.type === "text" ? child.value : "").join("");
        return {type: "element", tagName: display ? "div" : "span", properties: {
          [display ? "data-content-type" : "data-inline-content-type"]: display ? "formulaBlock" : "inlineFormula",
          "data-source": source,
        }, children: [{type: "text", value: source}]};
      }
      visit(node); return node;
    }) as Element["children"];
  }
  visit(tree);
  sanitizeMarkdownHast(tree);
  return found ? toHtml(tree) : null;
}
