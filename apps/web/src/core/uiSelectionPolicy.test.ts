/** @vitest-environment happy-dom */

import { describe, expect, it } from "vitest";

import { allowsAppTextSelection } from "./uiSelectionPolicy";

describe("application text-selection policy", () => {
  it("blocks ordinary application chrome and interactive labels", () => {
    const button = document.createElement("button");
    const label = document.createElement("span");
    button.append(label);

    expect(allowsAppTextSelection(button)).toBe(false);
    expect(allowsAppTextSelection(label)).toBe(false);
    expect(allowsAppTextSelection(null)).toBe(false);
  });

  it("preserves inputs, editors, PDF text, and quoted reading content", () => {
    for (const element of [
      document.createElement("input"),
      document.createElement("textarea"),
      Object.assign(document.createElement("div"), { contentEditable: "true" }),
      Object.assign(document.createElement("div"), { className: "textLayer" }),
      Object.assign(document.createElement("div"), { className: "cm-content" }),
      Object.assign(document.createElement("div"), { className: "bn-editor" }),
      document.createElement("blockquote"),
      document.createElement("q"),
    ]) {
      document.body.append(element);
      expect(allowsAppTextSelection(element)).toBe(true);
      element.remove();
    }
  });

  it("uses the nearest selectable ancestor for nested text nodes", () => {
    const quote = document.createElement("blockquote");
    const text = document.createTextNode("可复制的原文");
    quote.append(text);
    document.body.append(quote);

    expect(allowsAppTextSelection(text)).toBe(true);
    quote.remove();
  });
});
