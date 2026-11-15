// @vitest-environment happy-dom
import { describe, expect, it, vi } from "vitest";
import { pdfAutoLinkRanges } from "./pdfAutoLinks";
import { PdfPageCharacterStore } from "./pdfPageCharacterModel";
import { afterPdfCharacter, beforePdfCharacter, createPdfCharacterSelection } from "./pdfPageCharacterSelection";

function model(text: string) {
  const page = document.createElement("div");
  page.dataset.pageNumber = "1";
  page.getBoundingClientRect = () => new DOMRect(200, 300, 800, 1000);
  const store = new PdfPageCharacterStore();
  store.setPageCharacters(0, Array.from(text, (value, index) => ({
    value, rect: { x: index * .005, y: .2, width: .005, height: .02 }, rotation: 0,
  })));
  return store.syncPage(page)!;
}

describe("glyph-based PDF automatic links", () => {
  it("uses the same character selection geometry after a Unicode prefix and between adjacent emails", () => {
    const input = model("📖 Author: first@example.org, second@example.net;");
    const range = vi.spyOn(Range.prototype, "getBoundingClientRect").mockImplementation(() => { throw new Error("DOM geometry is forbidden"); });
    try {
      const links = pdfAutoLinkRanges(input);
      expect(links.map((link) => link.url)).toEqual(["mailto:first@example.org", "mailto:second@example.net"]);
      for (const [index, email] of ["first@example.org", "second@example.net"].entries()) {
        const start = Array.from(input.text.slice(0, input.text.indexOf(email))).length;
        const selection = createPdfCharacterSelection(input, beforePdfCharacter(start), afterPdfCharacter(start + email.length - 1));
        expect(links[index]!.rects).toEqual(selection!.rects);
      }
    } finally { range.mockRestore(); }
  });

  it("recognizes safe web/mail links, trims prose punctuation and retains balanced parentheses", () => {
    const links = pdfAutoLinkRanges(model("https://example.org/a(b). www.example.net; mailto:a@example.org"));
    expect(links.map((link) => link.url)).toEqual(["https://example.org/a(b)", "https://www.example.net/", "mailto:a@example.org"]);
    expect(pdfAutoLinkRanges(model("javascript:alert(1) data:text/plain,x file:///tmp/a a@127.0.0.1"))).toEqual([]);
  });
});

it.each(["replaced document", "replaced page", "missing glyphs"])("does not inject links after %s", async (scenario) => {
  const { bindPdfAutoLinks } = await import("./pdfAutoLinks");
  let handler: ((event: { pageNumber: number }) => void) | undefined;
  const off = vi.fn();
  const bus = { on: (_name: string, callback: typeof handler) => { handler = callback; }, off };
  const inject = vi.fn();
  const page = { annotationLayer: { injectLinkAnnotations: inject }, div: document.createElement("div") };
  const viewer = { getPageView: () => page };
  let current = true;
  let resolve: () => void = () => {};
  const pending = new Promise<void>((done) => { resolve = done; });
  const errors = vi.fn();
  const unbind = bindPdfAutoLinks(bus as unknown as Parameters<typeof bindPdfAutoLinks>[0],
    viewer as unknown as Parameters<typeof bindPdfAutoLinks>[1], new PdfPageCharacterStore(),
    () => pending, () => current, errors);
  handler!({ pageNumber: 1 });
  if (scenario === "replaced document") current = false;
  if (scenario === "replaced page") page.annotationLayer = { injectLinkAnnotations: inject };
  resolve();
  await pending;
  await Promise.resolve();
  expect(inject).not.toHaveBeenCalled();
  expect(errors).not.toHaveBeenCalled();
  unbind();
  expect(off).toHaveBeenCalledWith("annotationlayerrendered", handler);
});
