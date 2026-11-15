import type { PDFViewer } from "pdfjs-dist/legacy/web/pdf_viewer.mjs";
import { clampPdfScale, type PdfScale, type PdfScaleLimits } from "@research-assistant/core/pdf-view";

/** PDF.js 6.1 uses offset-parent coordinates for containerTopLeft, whereas
 * DOM wheel/gesture origins are client coordinates. Convert at the boundary,
 * using the live rectangle even after a sidebar or retained host has moved. */
export function pdfViewerZoomOrigin(viewer: PDFViewer, origin: readonly [number, number]): number[] {
  const bounds = viewer.container.getBoundingClientRect();
  const [top, left] = viewer.containerTopLeft;
  return [origin[0] - bounds.left - viewer.container.clientLeft + (left ?? 0),
    origin[1] - bounds.top - viewer.container.clientTop + (top ?? 0)];
}

/** The shared flex layout has its own gutters, in addition to PDF.js page
 * borders. Its stock 40px allowance does not describe that outer width. */
export function setPdfViewerScale(viewer: PDFViewer, scale: PdfScale, limits: PdfScaleLimits): void {
  if (scale.kind !== "page-width") {
    viewer.currentScaleValue = scale.kind === "numeric" ? String(clampPdfScale(scale.value, limits)) : scale.kind;
    return;
  }
  const page = viewer.getPageView(viewer.currentPageNumber - 1) as
    { readonly width: number; readonly scale: number; readonly div: HTMLDivElement } | undefined;
  if (!page || !viewer.viewer || viewer.container.clientWidth <= 0 || page.width <= 0) return;
  const layout = getComputedStyle(viewer.viewer);
  const paper = getComputedStyle(page.div);
  const px = (value: string): number => Number.parseFloat(value) || 0;
  const outerWidth = px(layout.paddingLeft) + px(layout.paddingRight)
    + px(paper.borderLeftWidth) + px(paper.borderRightWidth)
    + px(paper.marginLeft) + px(paper.marginRight);
  // Reserve one CSS pixel for PDF.js dimension rounding at fractional scales.
  const available = Math.floor(viewer.container.clientWidth - outerWidth) - 1;
  if (available <= 0) return;
  viewer.currentScaleValue = String(available / page.width * page.scale);
  viewer.container.scrollLeft = 0;
}
