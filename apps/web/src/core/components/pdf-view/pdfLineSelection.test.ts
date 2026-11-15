// @vitest-environment happy-dom
import { expect, it } from "vitest";
import { PdfPageCharacterStore } from "./pdfPageCharacterModel";
import { selectPdfLines } from "./pdfLineSelection";

it("selects inclusive PDF coordinates using worker glyphs and rejects stale positions", () => {
  const page = document.createElement("div"); page.dataset.pageNumber = "3";
  page.getBoundingClientRect = () => new DOMRect(0, 0, 600, 800);
  const store = new PdfPageCharacterStore();
  store.setPageCharacters(2, Array.from("A😀BC").map((value, index) => ({ value, rect: {x: .1 + index * .01, y: .1, width: .01, height: .02}, rotation: 0})));
  const model = store.syncPage(page)!;
  expect(selectPdfLines(model, {startLine: 1, startColumn: 2, endLine: 1, endColumn: 3})?.text).toBe("😀B");
  expect(selectPdfLines(model, {startLine: 2, startColumn: 1, endLine: 2, endColumn: 2})).toBeUndefined();
  expect(selectPdfLines(model, {startLine: 1, startColumn: 4, endLine: 1, endColumn: 2})).toBeUndefined();
});
