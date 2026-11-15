import {getMarkRange, posToDOMRect} from "@tiptap/core";
import type {LinkToolbarExtension} from "@blocknote/core/extensions";
import type {EditorView} from "@tiptap/pm/view";

type LinkAtElement = ReturnType<ReturnType<ReturnType<typeof LinkToolbarExtension>>["getLinkAtElement"]>;

/** Pointer hits identify a marked range, not the marks inherited by a caret.
 * Keep inclusive:false and the editor's selection lookup untouched. */
export function linkHoverRange(view: EditorView, element: HTMLElement): LinkAtElement {
  const anchor = element.closest<HTMLAnchorElement>("a[href]");
  if (!anchor || !view.dom.contains(anchor)) return undefined;
  const href = anchor.getAttribute("href"), type = view.state.schema.marks.link;
  if (!href || !type) return undefined;
  const range = getMarkRange(view.state.doc.resolve(view.posAtDOM(anchor, 0)), type, {href});
  if (!range) return undefined;
  return {
    range,
    mark: {attrs: {href}},
    text: view.state.doc.textBetween(range.from, range.to),
    get position() {return posToDOMRect(view, range.from, range.to).toJSON() as DOMRect;},
  };
}
