import { ChangeSet, type TokenEncoder } from "@tiptap/pm/changeset";
import type { Node } from "@tiptap/pm/model";
import { StepMap } from "@tiptap/pm/transform";
import { diff } from "@codemirror/merge";

/** PM's token diff retains real document positions. Inline leaf nodes encode
 * their complete attributes, rather than all looking like the same U+FFFC. */
const encoder: TokenEncoder<string | number> = {
  encodeCharacter: char => char,
  encodeNodeStart: node => JSON.stringify(["node", node.type.name, node.attrs, node.marks.map(mark => mark.toJSON())]),
  encodeNodeEnd: node => JSON.stringify(["end", node.type.name]),
  compareTokens: (a, b) => a === b,
};
interface InlineChange {
  readonly fromA: number;
  readonly toA: number;
  readonly fromB: number;
  readonly toB: number;
}

export function noteInlineChanges(before: Node | undefined, after: Node | undefined): readonly InlineChange[] {
  if (!before?.inlineContent || !after?.inlineContent || before.type !== after.type) {
    return [{ fromA: 0, toA: before?.content.size ?? 0, fromB: 0, toB: after?.content.size ?? 0 }];
  }
  let hasAtoms = false;
  before.forEach(node => { if (!node.isText) hasAtoms = true; });
  after.forEach(node => { if (!node.isText) hasAtoms = true; });
  // Retain the existing bounded text matcher for ordinary text containers.
  if (!hasAtoms) return diff(before.textContent, after.textContent, { scanLimit: 1000, timeout: 50 });
  return ChangeSet.create(before, undefined, encoder).addSteps(after,
    [new StepMap([0, before.content.size, after.content.size])], null).changes;
}

/** Serialized old content is presentation DOM. Stop at a whole atom wrapper;
 * never traverse or wrap its KaTeX/accessibility descendants as ordinary text. */
export function highlightOldInline(content: HTMLElement, before: Node | undefined, after: Node | undefined): void {
  const ranges = noteInlineChanges(before, after).filter(range => range.fromA < range.toA);
  const atomSizes = new Map<string, number>();
  before?.forEach(node => { if (node.isAtom && !node.isText) atomSizes.set(node.type.name, node.nodeSize); });
  const texts: { node: Text; from: number; to: number }[] = [];
  let offset = 0;
  const walk = (node: globalThis.Node) => {
    if (node instanceof Text) {
      texts.push({ node, from: offset, to: offset + node.length });
      offset += node.length;
      return;
    }
    if (!(node instanceof HTMLElement)) return;
    const size = atomSizes.get(node.getAttribute("data-inline-content-type") ?? "");
    if (size !== undefined) {
      if (ranges.some(range => range.fromA < offset + size && offset < range.toA)) {
        node.classList.add("note-diff-deleted-atom");
      }
      offset += size;
      return;
    }
    if (node instanceof HTMLBRElement) { offset++; return; }
    Array.from(node.childNodes).forEach(walk);
  };
  Array.from(content.childNodes).forEach(walk);
  for (const { node, from, to } of texts) {
    const overlaps = ranges.filter(range => range.fromA < to && range.toA > from);
    if (!overlaps.length) continue;
    const fragment = content.ownerDocument.createDocumentFragment();
    let cursor = from;
    for (const range of overlaps) {
      const start = Math.max(from, range.fromA), end = Math.min(to, range.toA);
      fragment.append(node.data.slice(cursor - from, start - from));
      const span = content.ownerDocument.createElement("span");
      span.className = "note-diff-deleted-text";
      span.textContent = node.data.slice(start - from, end - from);
      fragment.append(span);
      cursor = end;
    }
    fragment.append(node.data.slice(cursor - from));
    node.replaceWith(fragment);
  }
}
