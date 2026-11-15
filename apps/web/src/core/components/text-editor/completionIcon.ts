/**
 * SVG paths from lucide-react 1.28.0: braces, sigma and file-text (ISC).
 * Copyright (c) 2026 Lucide Icons and Contributors.
 * Full notice: public/licenses/LUCIDE_COMPLETION_LICENSE.txt.
 * Native DOM rendering avoids one React root or a font glyph per candidate.
 */
import type { Completion } from "@codemirror/autocomplete";
import type { EditorState } from "@codemirror/state";
import type { EditorView } from "@codemirror/view";

const braces = [
  "M8 3H7a2 2 0 0 0-2 2v5a2 2 0 0 1-2 2 2 2 0 0 1 2 2v5c0 1.1.9 2 2 2h1",
  "M16 21h1a2 2 0 0 0 2-2v-5c0-1.1.9-2 2-2a2 2 0 0 1-2-2V5a2 2 0 0 0-2-2h-1",
];
const sigma = ["M18 7V5a1 1 0 0 0-1-1H6.5a.5.5 0 0 0-.4.8l4.5 6a2 2 0 0 1 0 2.4l-4.5 6a.5.5 0 0 0 .4.8H17a1 1 0 0 0 1-1v-2"];
const file = [
  "M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z",
  "M14 2v5a1 1 0 0 0 1 1h5", "M10 9H8", "M16 13H8", "M16 17H8",
];

export function completionIcon(completion: Completion, _state: EditorState, view: EditorView): HTMLElement {
  const doc = view.dom.ownerDocument;
  const icon = doc.createElement("span");
  // Do not use CM's type classes: their ::after rules still render Unicode icons.
  icon.className = "cm-completionIcon editor-completion-icon";
  icon.setAttribute("aria-hidden", "true");
  const svg = doc.createElementNS("http://www.w3.org/2000/svg", "svg");
  for (const [name, value] of Object.entries({ viewBox: "0 0 24 24", width: "1em", height: "1em",
    fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round", focusable: "false" })) svg.setAttribute(name, value);
  const types = completion.type?.split(/\s+/u) ?? [];
  const paths = types.some(type => type === "function" || type === "method") ? braces
    : types.some(type => type === "keyword" || type === "variable" || type === "constant") ? sigma : file;
  for (const d of paths) {
    const path = doc.createElementNS("http://www.w3.org/2000/svg", "path");
    path.setAttribute("d", d); svg.append(path);
  }
  icon.append(svg);
  return icon;
}
