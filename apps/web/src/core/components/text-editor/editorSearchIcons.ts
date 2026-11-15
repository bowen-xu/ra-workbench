import { createElement, Fragment } from "react";
import { createPortal } from "react-dom";
import { createRoot } from "react-dom/client";
import { ArrowDown, ArrowUp, ChevronRight, ListFilter, Replace, ReplaceAll, X } from "lucide-react";
import { ControlHint } from "../ControlHint";

export type SearchIcon = "chevron" | "up" | "down" | "close" | "selection" | "replace" | "replaceAll";
const components = { chevron: ChevronRight, up: ArrowUp, down: ArrowDown, close: X, selection: ListFilter, replace: Replace, replaceAll: ReplaceAll };

/** One React owner renders the application's Lucide icons into the native CM panel. */
export function createSearchIcons() {
  const slots: { name: SearchIcon; node: HTMLSpanElement }[] = [];
  const hints: { button: HTMLButtonElement; shortcut?: string; contents?: ChildNode[] }[] = [];
  const root = createRoot(document.createElement("div"));
  return {
    icon(name: SearchIcon): HTMLSpanElement {
      const node = document.createElement("span"); node.className = "editor-search-icon"; node.setAttribute("aria-hidden", "true");
      slots.push({ name, node }); return node;
    },
    hint(button: HTMLButtonElement, shortcut?: string) {
      hints.push({button, ...(shortcut ? {shortcut} : {})});
    },
    mount() {
      for (const hint of hints) hint.contents ??= [...hint.button.childNodes];
      root.render(createElement(Fragment, null,
        ...slots.map(({ name, node }, index) => createPortal(createElement(components[name], { size: "1em", strokeWidth: 1.6 }), node, `icon-${index}`)),
        ...hints.map((hint, index) => createPortal(createElement(ControlHint, {
          content: hint.button.getAttribute("aria-label"), ...(hint.shortcut ? {shortcut: hint.shortcut} : {}),
          children: createElement("span", {className: "editor-search-hint-anchor", ref: (node: HTMLSpanElement | null) => {
            if (node && hint.contents) node.append(...hint.contents);
          }}),
        }), hint.button, `hint-${index}`)),
      ));
    },
    destroy() { queueMicrotask(() => root.unmount()); },
  };
}
