import { describe, expect, it, vi } from "vitest";

import {
  supportsPdfViewerResourceApi,
  trimPdfPageBuffer,
  type PdfCachedPageView,
} from "./pdfResourcePolicy";

function pageView(id: number): PdfCachedPageView {
  return { id, destroy: vi.fn() };
}

describe("PDF resource policy", () => {
  it("keeps visible pages, their neighbors, and two recent offscreen pages", () => {
    const pages = Array.from({ length: 9 }, (_, index) => pageView(index + 1));
    const viewer = {
      getCachedPageViews: () => new Set(pages),
      _getVisiblePages: () => ({ ids: new Set([5, 6]) }),
    };
    const result = trimPdfPageBuffer(viewer, 2);
    expect(result.destroyedPageIds).toEqual([1, 2, 3]);
    expect(result.retainedPageIds).toEqual([4, 5, 6, 7, 8, 9]);
    expect(pages.filter((page) => vi.mocked(page.destroy).mock.calls.length > 0).map((page) => page.id))
      .toEqual([1, 2, 3]);
  });

  it("retains only visible pages and neighbors when trimming background memory", () => {
    const pages = Array.from({ length: 9 }, (_, index) => pageView(index + 1));
    const result = trimPdfPageBuffer({
      getCachedPageViews: () => new Set(pages),
      _getVisiblePages: () => ({ ids: new Set([5, 6]) }),
    }, 0);
    expect(result.destroyedPageIds).toEqual([1, 2, 3, 8, 9]);
    expect(result.retainedPageIds).toEqual([4, 5, 6, 7]);
  });

  it("guards the private PDF.js visibility hook", () => {
    expect(supportsPdfViewerResourceApi({
      getCachedPageViews: () => new Set(),
      _getVisiblePages: () => ({ ids: new Set() }),
    })).toBe(true);
    expect(supportsPdfViewerResourceApi({ getCachedPageViews: () => new Set() })).toBe(false);
  });

  it("does not destroy cached pages before PDF.js reports a visible set", () => {
    const pages = [pageView(1), pageView(2), pageView(3)];
    const result = trimPdfPageBuffer({
      getCachedPageViews: () => new Set(pages),
      _getVisiblePages: () => ({ ids: new Set() }),
    }, 0);
    expect(result).toEqual({ destroyedPageIds: [], retainedPageIds: [1, 2, 3] });
    expect(pages.every((page) => vi.mocked(page.destroy).mock.calls.length === 0)).toBe(true);
  });
});
