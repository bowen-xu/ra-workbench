// @vitest-environment happy-dom
import { afterEach, expect, it } from "vitest";
import { readFileSync } from "node:fs";

const highlightStyles = readFileSync("apps/web/src/core/components/editor-diff/EditorDiffHighlight.css", "utf8");

const mounted: HTMLElement[] = [];
afterEach(() => { for (const element of mounted.splice(0)) element.remove(); });

it.each([
  ["note-diff-deleted-blocks hbs-note-incoming", "note-diff-deleted-blocks"],
  ["note-diff-deleted-content note-diff-deleted-tableCell hbs-note-incoming", "note-diff-deleted-content"],
  ["note-diff-deleted-content note-diff-deleted-codeBlock hbs-note-incoming", "note-diff-deleted-content"],
  ["note-diff-deleted-content note-diff-deleted-quote hbs-note-incoming", "note-diff-deleted-content"],
  ["editor-diff-removed-block editor-diff-incoming-block", "editor-diff-removed-block"],
  ["structured-card-diff-before hbs-structured-incoming", "structured-card-diff-before"],
])("paints incoming consistently when its renderer reuses old layout: %s", (incomingClasses, oldClasses) => {
  const style = document.createElement("style");
  style.textContent = highlightStyles;
  const surface = document.createElement("section");
  surface.className = "editor-diff-highlight";
  const incoming = document.createElement("div"), old = document.createElement("div");
  incoming.className = incomingClasses;
  old.className = oldClasses;
  surface.append(old, incoming);
  document.head.append(style); document.body.append(surface); mounted.push(style, surface);

  expect(getComputedStyle(incoming).borderLeftColor).toBe("#438dcc");
  expect(getComputedStyle(incoming).backgroundColor).toBe("#e7f1fc");
  expect(getComputedStyle(incoming).color).toBe("#245781");
  expect(getComputedStyle(old).borderLeftColor).toBe("#e43");
});
