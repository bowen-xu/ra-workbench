import type { PDFViewer } from "pdfjs-dist/legacy/web/pdf_viewer.mjs";
import type { PDFDocumentProxy } from "pdfjs-dist/legacy/build/pdf.mjs";
import type { PdfPagePoint, PdfRect as SourceRect } from "@research-assistant/core/pdf-view";
import { pdfPageContentRect } from "./pdfPageCharacterModel";

export function pdfPagePoint(viewerElement: HTMLElement | undefined, viewer: PDFViewer | undefined, clientX: number, clientY: number): PdfPagePoint | undefined {
    const page = [...(viewerElement?.querySelectorAll<HTMLElement>(".page[data-page-number]") ?? [])]
      .find((element) => { const r = pdfPageContentRect(element); return clientX >= r.left && clientX <= r.right && clientY >= r.top && clientY <= r.bottom; });
    if (!page || !viewer) return;
    const pageIndex = Number(page.dataset.pageNumber) - 1;
    const pageView = viewer.getPageView(pageIndex);
    const viewport = pageView?.viewport;
    if (!viewport || !pageView?.pdfPage || !pageView.canvas) return;
    const bounds = pdfPageContentRect(page);
    const [x, y] = viewport.convertToPdfPoint((clientX - bounds.left) * viewport.width / bounds.width, (clientY - bounds.top) * viewport.height / bounds.height);
    const box = pageView.pdfPage.view;
    return { pageIndex, x: x!, y: y!, pageBox: [box[0]!, box[1]!, box[2]!, box[3]!] };
  }

export function pdfReadingPoint(container: HTMLElement | undefined, viewerElement: HTMLElement | undefined, viewer: PDFViewer | undefined, offsetTopCssPx = 72): PdfPagePoint | undefined {
    const stage = container?.getBoundingClientRect();
    if (!stage || stage.width <= 0 || stage.height <= 0) return;
    for (const page of viewerElement?.querySelectorAll<HTMLElement>(".page[data-page-number]") ?? []) {
      const r = pdfPageContentRect(page);
      const left = Math.max(r.left, stage.left), right = Math.min(r.right, stage.right);
      const top = Math.max(r.top, stage.top), bottom = Math.min(r.bottom, stage.bottom);
      if (right > left && bottom > top) return pdfPagePoint(viewerElement, viewer, (left + right) / 2, Math.min(top + offsetTopCssPx, bottom - .5));
    }
  }

export async function loadPdfPageBox(document: PDFDocumentProxy | undefined, isCurrent: () => boolean, pageIndex: number): Promise<readonly [number, number, number, number] | undefined> {
    if (!document || pageIndex < 0 || pageIndex >= document.numPages) return;
    const page = await document.getPage(pageIndex + 1);
    if (!isCurrent()) return;
    return [page.view[0]!, page.view[1]!, page.view[2]!, page.view[3]!];
  }

export async function normalizedPdfRect(document: PDFDocumentProxy | undefined, isCurrent: () => boolean, pageIndex: number, rect: SourceRect): Promise<SourceRect | undefined> {
    if (!document || pageIndex < 0 || pageIndex >= document.numPages) return;
    const page = await document.getPage(pageIndex + 1);
    if (!isCurrent()) return;
    const viewport = page.getViewport({ scale: 1 });
    const [x1, y1] = viewport.convertToViewportPoint(rect.x, rect.y);
    const [x2, y2] = viewport.convertToViewportPoint(rect.x + rect.width, rect.y + rect.height);
    return { x: Math.min(x1!, x2!) / viewport.width, y: Math.min(y1!, y2!) / viewport.height, width: Math.abs(x2! - x1!) / viewport.width, height: Math.abs(y2! - y1!) / viewport.height };
  }

export function clientRectForPageRect(viewerElement: HTMLElement | undefined, pageIndex: number, rect: SourceRect): DOMRect | undefined {
    const page = viewerElement?.querySelector<HTMLElement>(`.page[data-page-number="${pageIndex + 1}"]`);
    if (page === undefined || page === null) return undefined;
    const bounds = pdfPageContentRect(page);
    return new DOMRect(
      bounds.left + rect.x * bounds.width,
      bounds.top + rect.y * bounds.height,
      rect.width * bounds.width,
      rect.height * bounds.height,
    );
  }

/** Tiny normalized rectangle locates a point; its UI icon keeps a fixed CSS size. */
export function normalizedPointRect(bounds: DOMRect, clientX: number, clientY: number): SourceRect | undefined {
  if (bounds.width <= 0 || bounds.height <= 0 || !Number.isFinite(clientX) || !Number.isFinite(clientY)
    || clientX < bounds.left || clientX > bounds.right || clientY < bounds.top || clientY > bounds.bottom) return;
  const size = 0.00001;
  return { x: Math.min(1 - size, (clientX - bounds.left) / bounds.width), y: Math.min(1 - size, (clientY - bounds.top) / bounds.height), width: size, height: size };
}

