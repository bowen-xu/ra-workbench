// @vitest-environment happy-dom

import { afterEach, describe, expect, it } from "vitest";

import {
  PdfPageCharacterStore,
  type PdfSourceCharacter,
} from "./pdfPageCharacterModel";
import {
  afterPdfCharacter,
  beforePdfCharacter,
  createPdfCharacterSelection,
  createPdfMultiPageCharacterSelection,
  expandPdfCharacterRange,
  hitTestPdfCharacterOffset,
  normalizePdfCharacterRange,
} from "./pdfPageCharacterSelection";

function pageFixture(pageNumber = 1, rect = new DOMRect(10, 20, 100, 200)): HTMLElement {
  const page = document.createElement("section");
  page.className = "page";
  page.dataset.pageNumber = String(pageNumber);
  page.getBoundingClientRect = () => rect;
  document.body.append(page);
  return page;
}

function sourceLine(
  text: string,
  x: number,
  y: number,
  width = 0.05,
  height = 0.06,
  options: { readonly rotation?: number; readonly lineBreakAfter?: boolean } = {},
): readonly PdfSourceCharacter[] {
  return Array.from(text).map((value, index, values) => ({
    value,
    rect: { x: x + index * width, y, width, height },
    rotation: options.rotation ?? 0,
    lineBreakAfter: options.lineBreakAfter === true && index === values.length - 1,
  }));
}

function modelFor(chars: readonly PdfSourceCharacter[]): NonNullable<ReturnType<PdfPageCharacterStore["syncPage"]>> {
  const page = pageFixture();
  const store = new PdfPageCharacterStore();
  store.setPageCharacters(0, chars);
  const model = store.syncPage(page);
  if (model === undefined) throw new Error("fixture did not build a page model");
  return model;
}

afterEach(() => document.body.replaceChildren());

describe("PDF per-page character model", () => {
  it("releases source geometry and the derived model with a destroyed page view", () => {
    const page = pageFixture();
    const store = new PdfPageCharacterStore();
    store.setPageCharacters(0, sourceLine("A", 0.1, 0.1));
    expect(store.syncPage(page)).toBeDefined();
    store.releasePage(0);
    expect(store.hasPageCharacters(0)).toBe(false);
    expect(store.get(0)).toBeUndefined();
  });

  it("uses only worker-provided PDF character coordinates and never falls back to DOM Range geometry", () => {
    const page = pageFixture();
    const textLayer = document.createElement("div");
    textLayer.className = "textLayer";
    textLayer.textContent = "misaligned browser text";
    page.append(textLayer);
    const store = new PdfPageCharacterStore();
    expect(store.syncPage(page)).toBeUndefined();

    store.setPageCharacters(0, sourceLine("PDF", 0.2, 0.1, 0.08, 0.05));
    const model = store.syncPage(page);
    expect(model?.text).toBe("PDF");
    expect(model?.chars[0]?.rect).toEqual({ x: 0.2, y: 0.1, width: 0.08, height: 0.05 });
    const rect = createPdfCharacterSelection(
      model as NonNullable<typeof model>,
      beforePdfCharacter(0),
      afterPdfCharacter(2),
    )?.rects[0];
    expect(rect).toMatchObject({ x: 0.2, y: 0.1, width: 0.24 });
    expect(rect?.height).toBeCloseTo(0.05);
  });

  it("preserves worker reading order and Unicode glyph values", () => {
    const model = modelFor([
      ...sourceLine("A😀B", 0.1, 0.1, 0.1),
      ...sourceLine("C", 0.1, 0.25, 0.1),
    ]);
    expect(model.text).toBe("A😀B\nC");
    expect(model.chars.map((char) => char.value)).toEqual(["A", "😀", "B", "\n", "C"]);
    expect(model.lines).toHaveLength(2);
  });

  it("normalizes forward and reverse selections identically and trims boundary whitespace", () => {
    const model = modelFor(sourceLine("alpha beta", 0.05, 0.1, 0.04));
    expect(normalizePdfCharacterRange(
      model,
      beforePdfCharacter(0),
      afterPdfCharacter(model.chars.length - 1),
    )).toEqual({ start: 0, end: 10 });
    const forward = createPdfCharacterSelection(model, beforePdfCharacter(0), afterPdfCharacter(model.chars.length - 1));
    const reverse = createPdfCharacterSelection(model, afterPdfCharacter(model.chars.length - 1), beforePdfCharacter(0));
    expect(forward?.text).toBe("alpha beta");
    expect(reverse?.segments).toEqual(forward?.segments);
    expect(reverse?.focusPageIndex).toBe(forward?.focusPageIndex);
    expect(reverse?.focusRect).not.toEqual(forward?.focusRect);
    expect(hitTestPdfCharacterOffset(model, 10 + 0.23 * 100, 20 + 0.12 * 200)).toEqual({ index: 4, offset: 1 });
  });

  it("uses character before/after offsets and the PDF content box for pointer hit testing", () => {
    const page = pageFixture(1, new DOMRect(10, 20, 118, 218));
    const canvasWrapper = document.createElement("div");
    canvasWrapper.className = "canvasWrapper";
    canvasWrapper.getBoundingClientRect = () => new DOMRect(19, 29, 100, 200);
    page.append(canvasWrapper);
    const store = new PdfPageCharacterStore();
    store.setPageCharacters(0, sourceLine("AB", 0.1, 0.1, 0.1, 0.06));
    const model = store.syncPage(page);
    if (model === undefined) throw new Error("content-box fixture did not build a page model");

    expect(hitTestPdfCharacterOffset(model, 19 + 0.12 * 100, 29 + 0.12 * 200))
      .toEqual({ index: 0, offset: 0 });
    expect(hitTestPdfCharacterOffset(model, 19 + 0.18 * 100, 29 + 0.12 * 200))
      .toEqual({ index: 0, offset: 1 });
    expect(createPdfCharacterSelection(model, beforePdfCharacter(0), beforePdfCharacter(0))).toBeUndefined();
    expect(createPdfCharacterSelection(model, beforePdfCharacter(0), afterPdfCharacter(0))?.lastClientRect)
      .toMatchObject({ left: 29, top: 49, width: 10, height: 12 });
  });

  it("derives before/after direction from adjacent RTL glyph progression", () => {
    const model = modelFor([
      { value: "א", rect: { x: 0.3, y: 0.1, width: 0.05, height: 0.06 }, rotation: 0 },
      { value: "ב", rect: { x: 0.25, y: 0.1, width: 0.05, height: 0.06 }, rotation: 0 },
    ]);
    expect(hitTestPdfCharacterOffset(model, 10 + 0.34 * 100, 20 + 0.12 * 200))
      .toEqual({ index: 0, offset: 0 });
    expect(hitTestPdfCharacterOffset(model, 10 + 0.31 * 100, 20 + 0.12 * 200))
      .toEqual({ index: 0, offset: 1 });
  });

  it("expands Unicode words, lines, and all-page ranges without splitting glyph values", () => {
    const model = modelFor([
      { value: "ffi", rect: { x: 0.1, y: 0.1, width: 0.08, height: 0.05 }, rotation: 0 },
      { value: "e", rect: { x: 0.18, y: 0.1, width: 0.04, height: 0.05 }, rotation: 0 },
      { value: "\u0301", rect: { x: 0.18, y: 0.1, width: 0.04, height: 0.05 }, rotation: 0 },
      ...sourceLine(" שלום", 0.22, 0.1, 0.04, 0.06, { lineBreakAfter: true }),
      ...sourceLine("中文", 0.1, 0.25, 0.06),
    ]);
    const ligatureWord = expandPdfCharacterRange(model, 0, "word");
    expect(createPdfCharacterSelection(
      model,
      beforePdfCharacter(ligatureWord?.start ?? 0),
      afterPdfCharacter((ligatureWord?.end ?? 1) - 1),
    )?.text)
      .toBe("ffié");
    const rtlIndex = model.chars.findIndex((character) => character.value === "ש");
    const rtlWord = expandPdfCharacterRange(model, rtlIndex, "word");
    expect(createPdfCharacterSelection(
      model,
      beforePdfCharacter(rtlWord?.start ?? rtlIndex),
      afterPdfCharacter((rtlWord?.end ?? rtlIndex + 1) - 1),
    )?.text)
      .toBe("שלום");
    const cjkIndex = model.chars.findIndex((character) => character.value === "中");
    const cjkLine = expandPdfCharacterRange(model, cjkIndex, "line");
    expect(createPdfCharacterSelection(
      model,
      beforePdfCharacter(cjkLine?.start ?? cjkIndex),
      afterPdfCharacter((cjkLine?.end ?? cjkIndex + 1) - 1),
    )?.text)
      .toBe("中文");
    expect(expandPdfCharacterRange(model, cjkIndex, "all")).toEqual({ start: 0, end: model.chars.length });
  });

  it("removes discretionary soft hyphens from exact selected text", () => {
    const model = modelFor(sourceLine("inter\u00adnational", 0.1, 0.1, 0.04));
    expect(createPdfCharacterSelection(model, beforePdfCharacter(0), afterPdfCharacter(model.chars.length - 1))?.text)
      .toBe("international");
  });

  it("creates identical reading-order segments for forward and reverse adjacent-page selection", () => {
    const store = new PdfPageCharacterStore();
    const firstPage = pageFixture(1, new DOMRect(10, 20, 100, 200));
    const secondPage = pageFixture(2, new DOMRect(10, 240, 100, 200));
    store.setPageCharacters(0, sourceLine("FIRST", 0.1, 0.1));
    store.setPageCharacters(1, sourceLine("SECOND", 0.1, 0.1));
    const first = store.syncPage(firstPage);
    const second = store.syncPage(secondPage);
    if (first === undefined || second === undefined) throw new Error("two-page fixture is incomplete");
    const forward = createPdfMultiPageCharacterSelection(first, beforePdfCharacter(2), second, afterPdfCharacter(2));
    const reverse = createPdfMultiPageCharacterSelection(second, afterPdfCharacter(2), first, beforePdfCharacter(2));
    expect(forward?.segments.map((segment) => [segment.pageIndex, segment.text])).toEqual([[0, "RST"], [1, "SEC"]]);
    expect(reverse?.segments.map((segment) => [segment.pageIndex, segment.text])).toEqual([[0, "RST"], [1, "SEC"]]);
    expect(forward?.segments.map(segment => segment.location)).toEqual([
      { startLine: 1, startColumn: 3, endLine: 1, endColumn: 5 },
      { startLine: 1, startColumn: 1, endLine: 1, endColumn: 3 },
    ]);
    expect(reverse?.segments.map(segment => segment.location)).toEqual(forward?.segments.map(segment => segment.location));
    expect(createPdfMultiPageCharacterSelection(
      first,
      beforePdfCharacter(0),
      { ...second, pageIndex: 2 },
      afterPdfCharacter(0),
    )).toBeUndefined();
  });

  it("records one-based start and inclusive end columns across page-local lines", () => {
    const model = modelFor([
      ...sourceLine("ABCD", .1, .1, .05, .06, { lineBreakAfter: true }),
      ...sourceLine("EFGH", .1, .4, .05, .06),
    ]);
    const first = model.chars.findIndex(character => character.value === "B");
    const last = model.chars.findIndex(character => character.value === "G");
    const selection = createPdfCharacterSelection(model, beforePdfCharacter(first), afterPdfCharacter(last));
    expect(selection?.segments[0]?.location).toEqual({ startLine: 1, startColumn: 2, endLine: 2, endColumn: 3 });
    const reverse = createPdfCharacterSelection(model, afterPdfCharacter(last), beforePdfCharacter(first));
    expect(reverse?.segments[0]?.location).toEqual(selection?.segments[0]?.location);
  });

  it("merges glyphs on a line while keeping columns and subsequent lines separate", () => {
    const model = modelFor([
      ...sourceLine("LEFT", 0.1, 0.1, 0.05),
      ...sourceLine("RIGHT", 0.65, 0.095, 0.05, 0.08),
      ...sourceLine("NEXT", 0.1, 0.25, 0.05),
    ]);
    const selection = createPdfCharacterSelection(model, beforePdfCharacter(0), afterPdfCharacter(model.chars.length - 1));
    expect(selection?.text).toBe("LEFT RIGHT\nNEXT");
    expect(selection?.rects).toHaveLength(3);
    const left = selection?.rects.find((rect) => rect.x === 0.1 && rect.y < 0.2);
    const right = selection?.rects.find((rect) => rect.x === 0.65);
    const next = selection?.rects.find((rect) => rect.y === 0.25);
    expect(left).toMatchObject({ x: 0.1, y: 0.1 });
    expect(left?.width).toBeCloseTo(0.2);
    expect(left?.height).toBeCloseTo(0.06);
    expect(right).toMatchObject({ x: 0.65, y: 0.095 });
    expect(right?.width).toBeCloseTo(0.25);
    expect(right?.height).toBeCloseTo(0.08);
    expect(next).toMatchObject({ x: 0.1, y: 0.25 });
    expect(next?.width).toBeCloseTo(0.2);
    expect(model.lines).toHaveLength(3);
  });

  it("keeps each selected line horizontally tight when another column shares its vertical band", () => {
    const leftFirst = sourceLine("SHORT", 0.08, 0.1, 0.035);
    const leftSecond = sourceLine("A LONGER LEFT LINE", 0.08, 0.2, 0.025);
    const rightFirst = sourceLine("RIGHT COLUMN WITH A TALLER FONT", 0.62, 0.09, 0.012, 0.09);
    const rightSecond = sourceLine("SECOND RIGHT LINE", 0.62, 0.19, 0.018, 0.08);
    const model = modelFor([
      ...leftFirst,
      ...leftSecond,
      ...rightFirst,
      ...rightSecond,
    ]);
    const lastLeftIndex = model.chars.reduce((last, character, index) => (
      character.rect.x < 0.6 && !/\s/u.test(character.value) ? index : last
    ), -1);
    const selection = createPdfCharacterSelection(
      model,
      beforePdfCharacter(0),
      afterPdfCharacter(lastLeftIndex),
    );

    expect(selection?.rects).toHaveLength(2);
    expect(selection?.rects.map((rect) => rect.x + rect.width))
      .toEqual(expect.arrayContaining([expect.closeTo(0.255, 5), expect.closeTo(0.53, 5)]));
    expect(Math.max(...(selection?.rects.map((rect) => rect.x + rect.width) ?? [])))
      .toBeLessThan(0.6);
  });

  it("uses a shared PDF inline box on the block axis", () => {
    const model = modelFor([
      { value: "A", rect: { x: 0.1, y: 0.12, width: 0.05, height: 0.04 }, rotation: 0 },
      { value: "g", rect: { x: 0.15, y: 0.1, width: 0.05, height: 0.08 }, rotation: 0 },
    ]);
    const selection = createPdfCharacterSelection(model, beforePdfCharacter(0), afterPdfCharacter(1));
    expect(model.chars[0]?.inlineRect.y).toBeCloseTo(0.1);
    expect(model.chars[0]?.inlineRect.height).toBeCloseTo(0.08);
    expect(selection?.rects[0]).toMatchObject({ x: 0.1, y: 0.1, width: 0.1 });
    expect(selection?.rects[0]?.height).toBeCloseTo(0.08);
  });

  it("uses the inline box width for vertical glyphs", () => {
    const model = modelFor([
      { value: "縦", rect: { x: 0.25, y: 0.1, width: 0.08, height: 0.05 }, rotation: 90 },
      { value: "書", rect: { x: 0.23, y: 0.15, width: 0.12, height: 0.05 }, rotation: 90 },
    ]);
    const selection = createPdfCharacterSelection(model, beforePdfCharacter(0), afterPdfCharacter(0));
    expect(selection?.rects[0]).toMatchObject({ x: 0.23, y: 0.1 });
    expect(selection?.rects[0]?.height).toBeCloseTo(0.05);
    expect(selection?.rects[0]?.width).toBeCloseTo(0.12);
  });

  it("reuses a current model and rebuilds after source, size, or page host changes", () => {
    const page = pageFixture();
    const store = new PdfPageCharacterStore();
    store.setPageCharacters(0, sourceLine("cache", 0.1, 0.1));
    const first = store.syncPage(page);
    expect(store.syncPage(page)?.version).toBe(first?.version);

    page.getBoundingClientRect = () => new DOMRect(10, 20, 200, 200);
    const resized = store.syncPage(page);
    expect(resized?.version).toBeGreaterThan(first?.version ?? 0);

    const replacement = pageFixture(1, new DOMRect(10, 20, 200, 200));
    const remounted = store.syncPage(replacement);
    expect(remounted?.version).toBeGreaterThan(resized?.version ?? 0);

    store.setPageCharacters(0, sourceLine("new", 0.2, 0.2));
    const refreshed = store.syncPage(replacement);
    expect(refreshed?.version).toBeGreaterThan(remounted?.version ?? 0);
    expect(refreshed?.text).toBe("new");
  });
});

it.each([90, 180, 270])("preserves contiguous text in a page rotated by %s degrees", (rotation) => {
  const text = "reader@example.org";
  const chars = Array.from(text, (value, index) => {
    const step = .02 * (rotation === 90 ? index : text.length - 1 - index);
    return { value, rect: rotation === 180
      ? { x: .1 + step, y: .2, width: .02, height: .03 }
      : { x: .2, y: .1 + step, width: .03, height: .02 }, rotation };
  });
  expect(modelFor(chars).text).toBe(text);
});
