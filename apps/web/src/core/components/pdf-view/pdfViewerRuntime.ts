import type { PDFViewer } from "pdfjs-dist/legacy/web/pdf_viewer.mjs";
import { setPdfViewerScale } from "./pdfViewerScale";
import { clampPdfScale, type PdfScaleLimits, type PdfLocation, type PdfScale, type PdfViewState } from "@research-assistant/core/pdf-view";
import type { PdfCachedPageView } from "./pdfResourcePolicy";
export interface PageChangingEvent {
  readonly pageNumber: number;
}

export interface ScaleChangingEvent {
  readonly scale: number;
}

export interface TextLayerRenderedEvent {
  readonly pageNumber: number;
}

export interface ViewAreaEvent {
  readonly location: {
    readonly pageNumber: number;
    readonly scale: number;
    readonly top?: number;
    readonly left?: number;
  };
}

export interface FindMatchesEvent {
  readonly matchesCount?: {
    readonly current: number;
    readonly total: number;
  };
}

export interface PdfRenderingQueueRuntime {
  renderHighestPriority(...args: readonly unknown[]): unknown;
}

export interface PdfViewerRuntime {
  readonly pagesCount: number;
  readonly renderingQueue: PdfRenderingQueueRuntime;
  getPageView(pageIndex: number): PdfCachedPageView | undefined;
  update(): void;
}

export interface PdfPageViewLocationRuntime extends PdfCachedPageView {
  readonly viewport?: {
    readonly width: number;
    readonly height: number;
    convertToPdfPoint(x: number, y: number): readonly [number, number];
  };
}

export interface PendingSecondNavigation {
  readonly location: PdfLocation;
  readonly pageIndex: number;
}

export function clampPageIndex(pageIndex: number, pageCount: number): number {
  if (!Number.isFinite(pageIndex) || pageCount <= 0) return 0;
  return Math.max(0, Math.min(pageCount - 1, Math.trunc(pageIndex)));
}


export function pdfScale(viewer: PDFViewer, limits: PdfScaleLimits): PdfScale {
  const value = viewer.currentScaleValue;
  if (value === "page-fit" || value === "page-width" || value === "page-height" || value === "auto") {
    return { kind: value };
  }
  return { kind: "numeric", value: clampPdfScale(viewer.currentScale, limits) };
}

export function pdfViewState(viewer: PDFViewer, location: ViewAreaEvent["location"], limits: PdfScaleLimits): PdfViewState {
  return {
    pageIndex: Math.max(0, location.pageNumber - 1),
    scale: pdfScale(viewer, limits),
    ...(location.top === undefined ? {} : { top: location.top }),
    ...(location.left === undefined ? {} : { left: location.left }),
    scrollMode: "vertical",
    spreadMode: "none",
  };
}

export function applyPdfLocation(viewer: PDFViewer | undefined, container: HTMLElement | undefined, limits: PdfScaleLimits, location: PdfLocation, pageIndex: number): void {
    if (viewer === undefined) return;
    if (location.scale !== undefined) {
      // Fit presets depend on the active page's dimensions and rotation.
      if (viewer.currentPageNumber !== pageIndex + 1) viewer.currentPageNumber = pageIndex + 1;
      setPdfViewerScale(viewer, location.scale, limits);
    }
    let left = location.left;
    let top = location.top;
    const segment = location.position?.segments.find((candidate) => candidate.pageIndex === pageIndex)
      ?? location.position?.segments[0];
    const firstRect = segment?.rects[0];
    if (firstRect !== undefined) {
      const runtime = viewer as unknown as PdfViewerRuntime;
      const pageView = runtime.getPageView(pageIndex) as PdfPageViewLocationRuntime | undefined;
      const viewport = pageView?.viewport;
      if (viewport !== undefined) {
        const center = location.alignment === "center" && container && segment?.rects.length;
        const x = center ? (Math.min(...segment.rects.map(rect => rect.x)) + Math.max(...segment.rects.map(rect => rect.x + rect.width))) / 2 : firstRect.x;
        const y = center ? (Math.min(...segment.rects.map(rect => rect.y)) + Math.max(...segment.rects.map(rect => rect.y + rect.height))) / 2 : firstRect.y;
        [left, top] = viewport.convertToPdfPoint(
          x * viewport.width - (center ? container.clientWidth / 2 : 0),
          y * viewport.height - (center ? container.clientHeight / 2 : 0),
        );
      }
    }
    const previousScrollBehavior = container?.style.scrollBehavior;
    if (location.behavior === "smooth" && container !== undefined) {
      container.style.scrollBehavior = "smooth";
    }
    if (left !== undefined || top !== undefined) {
      viewer.scrollPageIntoView({
        pageNumber: pageIndex + 1,
        destArray: [null, { name: "XYZ" }, left ?? null, top ?? null, null],
        ignoreDestinationZoom: true,
        allowNegativeOffset: location.alignment === "center",
      });
    } else {
      viewer.scrollPageIntoView({ pageNumber: pageIndex + 1 });
    }
    if (container && location.visibilityMargin) container.scrollTop = Math.max(0, container.scrollTop - location.visibilityMargin);
    if (location.behavior === "smooth" && container !== undefined) {
      window.requestAnimationFrame(() => {
        if (container !== undefined) container.style.scrollBehavior = previousScrollBehavior ?? "";
      });
    }
  }

