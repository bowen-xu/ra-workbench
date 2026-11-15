// @vitest-environment happy-dom
import { describe, expect, it } from "vitest";
import { PdfPageCharacterStore } from "./pdfPageCharacterModel";
import { selectPdfQuote } from "./pdfQuoteSelection";
function model(text: string) {
  const page = document.createElement("div"); page.dataset.pageNumber = "1";
  page.getBoundingClientRect = () => new DOMRect(0, 0, 600, 800);
  const store = new PdfPageCharacterStore();
  store.setPageCharacters(0, Array.from(text).map((value, index) => ({ value, rect: { x: .1 + index * .01, y: .1, width: .01, height: .02 }, rotation: 0 })));
  const result = store.syncPage(page);
  if (result === undefined) throw new Error("missing model");
  return result;
}
describe("source quotation to glyph selection", () => {
  it("selects exact worker characters despite whitespace and preserves Unicode offsets", () => {
    const selected = selectPdfQuote(model("A 😀 formal model."), "😀 formal   model");
    expect(selected?.text).toBe("😀 formal model"); expect(selected?.rects.length).toBeGreaterThan(0);
  });
  it("refuses ambiguous and fabricated matches", () => {
    expect(selectPdfQuote(model("model then model"), "model")).toBeUndefined();
    expect(selectPdfQuote(model("model then model"), "model", "then ")?.text).toBe("model");
    expect(selectPdfQuote(model("actual source"), "invented")).toBeUndefined();
  });
  it("maps expanded Latin ligatures back to their original glyphs without folding scientific notation", () => {
    expect(selectPdfQuote(model("A ﬁeld reﬁnes an oﬃce."), "field refines an office")?.text).toBe("ﬁeld reﬁnes an oﬃce");
    expect(selectPdfQuote(model("office"), "oﬃce")?.text).toBe("office");
    expect(selectPdfQuote(model("ﬁeld then field"), "field")).toBeUndefined();
    expect(selectPdfQuote(model("ﬁeld then field"), "field", "then ")?.text).toBe("field");
    expect(selectPdfQuote(model("x²"), "x2")).toBeUndefined();
    expect(selectPdfQuote(model("a-b"), "ab")).toBeUndefined();
  });
});
