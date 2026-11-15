// @vitest-environment happy-dom
import { afterEach, expect, it, vi } from "vitest";
import type { PDFViewer } from "pdfjs-dist/legacy/web/pdf_viewer.mjs";
import { pdfPagePoint, pdfReadingPoint } from "./pdfCoordinates";
import { writingPdfRect, writingReadingPosition } from "../../../../../../plugins/writing_projects/src/features/writing/preview/writingPdfCoordinates";

function setup() {
  const container = document.createElement("div"), viewer = document.createElement("div"); container.append(viewer);
  vi.spyOn(container, "getBoundingClientRect").mockReturnValue(new DOMRect(0, 0, 600, 700));
  const pages = [new DOMRect(-200, -400, 1200, 1600), new DOMRect(0, 1220, 600, 800)].map((bounds, index) => {
    const page = document.createElement("div"); page.className = "page"; page.dataset.pageNumber = String(index + 1); viewer.append(page);
    vi.spyOn(page, "getBoundingClientRect").mockReturnValue(bounds); return page;
  });
  const convert = vi.fn((x: number, y: number) => [x / 1.375 + 10, 820 - y / 1.375]);
  const runtime = { getPageView: () => ({ canvas: {}, pdfPage: { view: [10, 20, 610, 820] }, viewport: { width: 825, height: 1100, convertToPdfPoint: convert } }) } as unknown as PDFViewer;
  return { container, viewer, pages, runtime, convert };
}
afterEach(() => vi.restoreAllMocks());
it("uses the uppermost visible page, intersection center and 72 CSS pixels before viewport conversion", () => {
  const f = setup(); const point = pdfReadingPoint(f.container, f.viewer, f.runtime)!;
  expect(f.convert).toHaveBeenCalledWith(343.75, 324.5);
  expect(point).toEqual({ pageIndex: 0, x: 260, y: 584, pageBox: [10, 20, 610, 820] });
  expect(writingReadingPosition(point, { key: "draft", revision: "compile" })).toEqual({ key: "draft", revision: "compile", page: 1, x: 250, y: 236 });
});
it("clamps a short visible sliver and skips pages completely above the viewport", () => {
  const f = setup(); vi.mocked(f.pages[0]!.getBoundingClientRect).mockReturnValue(new DOMRect(0, -1580, 1200, 1600));
  pdfReadingPoint(f.container, f.viewer, f.runtime);
  expect(f.convert.mock.calls[0]?.[1]).toBeCloseTo(1599.5 * 1100 / 1600);
  vi.mocked(f.pages[0]!.getBoundingClientRect).mockReturnValue(new DOMRect(0, -2000, 600, 800));
  vi.mocked(f.pages[1]!.getBoundingClientRect).mockReturnValue(new DOMRect(0, 20, 600, 800));
  expect(pdfReadingPoint(f.container, f.viewer, f.runtime)?.pageIndex).toBe(1);
});
it("uses the viewport inverse transform for rotated pages rather than dividing PDF coordinates by scale", () => {
  const f = setup(); f.convert.mockImplementation((x, y) => [y / 1.375 + 10, x / 1.375 + 20]);
  const point = pdfPagePoint(f.viewer, f.runtime, 300, 72)!;
  expect(point.x).toBe(246); expect(point.y).toBe(270);
  expect(writingPdfRect({ x: 20, y: 30, width: 5, height: 8 }, [10, 20, 610, 820])).toEqual({ x: 30, y: 782, width: 5, height: 8 });
});
it("does not manufacture an anchor for a hidden preview or unloaded page", () => {
  const f = setup(); vi.mocked(f.container.getBoundingClientRect).mockReturnValue(new DOMRect());
  expect(pdfReadingPoint(f.container, f.viewer, f.runtime)).toBeUndefined();
  expect(pdfPagePoint(f.viewer, undefined, 100, 100)).toBeUndefined();
});

it("stores the same normalized point under zoom and clamps page edges", async () => {
  const { normalizedPointRect } = await import("./pdfCoordinates");
  expect(normalizedPointRect(new DOMRect(100, 200, 400, 600), 200, 500))
    .toEqual(normalizedPointRect(new DOMRect(0, -100, 800, 1200), 200, 500));
  const edge = normalizedPointRect(new DOMRect(0, 0, 400, 600), 400, 600)!;
  expect(edge.x + edge.width).toBe(1); expect(edge.y + edge.height).toBe(1);
  expect(normalizedPointRect(new DOMRect(0, 0, 400, 600), -1, 30)).toBeUndefined();
  expect(normalizedPointRect(new DOMRect(0, 0, 0, 0), 0, 0)).toBeUndefined();
});
