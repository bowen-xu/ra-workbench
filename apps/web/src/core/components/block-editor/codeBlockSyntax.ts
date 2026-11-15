import { StreamLanguage } from "@codemirror/language";
import { python } from "@codemirror/legacy-modes/mode/python";
import { javascript, typescript, json } from "@codemirror/legacy-modes/mode/javascript";
import { stex } from "@codemirror/legacy-modes/mode/stex";
import { classHighlighter, highlightTree, tagHighlighter, tags } from "@lezer/highlight";
import { Plugin } from "@tiptap/pm/state";
import { Decoration, DecorationSet } from "@tiptap/pm/view";
import type { Node } from "@tiptap/pm/model";

const languages = {
  python: StreamLanguage.define(python), javascript: StreamLanguage.define(javascript),
  typescript: StreamLanguage.define(typescript), json: StreamLanguage.define(json), latex: StreamLanguage.define(stex),
};
const aliases: Readonly<Record<string, keyof typeof languages>> = { py: "python", js: "javascript", ts: "typescript", tex: "latex" };
export const blockCodeLanguages = {
  text: { name: "纯文本" }, python: { name: "Python", aliases: ["py"] },
  javascript: { name: "JavaScript", aliases: ["js"] }, typescript: { name: "TypeScript", aliases: ["ts"] },
  json: { name: "JSON" }, latex: { name: "LaTeX", aliases: ["tex"] },
};
export interface CodeHighlightToken { readonly from: number; readonly to: number; readonly classes: string }
// Preserve builtin/function modifiers which classHighlighter drops.
const codeHighlighters = [tagHighlighter([
  { tag: tags.standard(tags.variableName), class: "tok-builtin" },
  { tag: [tags.function(tags.variableName), tags.function(tags.propertyName)], class: "tok-function" },
  { tag: tags.tagName, class: "tok-tagName" },
  { tag: tags.attributeName, class: "tok-propertyName" },
]), classHighlighter];

export function codeHighlightTokens(source: string, name: string): readonly CodeHighlightToken[] {
  const key = aliases[name] ?? name;
  const language = languages[key as keyof typeof languages];
  // Unsupported/very large blocks remain fully editable plaintext. Parsing is
  // presentation only and must never truncate source or stop a save.
  if (!language || source.length > 100_000) return [];
  const result: CodeHighlightToken[] = [];
  try { highlightTree(language.parser.parse(source), codeHighlighters, (from, to, classes) => result.push({ from, to, classes })); }
  catch { return []; }
  return result;
}

/** Same tokenizer for readonly review snapshots and live code nodes. */
export function highlightBlockCodeSnapshot(element: HTMLElement, language: string): void {
  const source = element.textContent ?? "", fragment = element.ownerDocument.createDocumentFragment();
  let position = 0;
  for (const token of codeHighlightTokens(source, language)) {
    fragment.append(source.slice(position, token.from));
    const span = element.ownerDocument.createElement("span");
    span.className = token.classes;
    span.textContent = source.slice(token.from, token.to);
    fragment.append(span); position = token.to;
  }
  fragment.append(source.slice(position)); element.replaceChildren(fragment);
}

export function createBlockCodeHighlightPlugin(): Plugin<DecorationSet> {
  const cache = new WeakMap<Node, readonly CodeHighlightToken[]>();
  const decorate = (document: Node) => {
    const decorations: Decoration[] = [];
    document.descendants((node, position) => {
      if (node.type.name !== "codeBlock") return true;
      const spans = cache.get(node) ?? codeHighlightTokens(node.textContent, String(node.attrs.language ?? "text"));
      cache.set(node, spans);
      for (const span of spans) decorations.push(Decoration.inline(position + 1 + span.from, position + 1 + span.to, { class: span.classes }));
      return false;
    });
    return DecorationSet.create(document, decorations);
  };
  return new Plugin<DecorationSet>({
    state: { init: (_, state) => decorate(state.doc), apply: (transaction, previous) => transaction.docChanged ? decorate(transaction.doc) : previous },
    props: { decorations(state) { return this.getState(state) ?? DecorationSet.empty; } },
  });
}
