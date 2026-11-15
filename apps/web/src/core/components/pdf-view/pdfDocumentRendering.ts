import type { PdfOutlineEntry } from "./pdfViewTypes";
import type { PDFDocumentProxy } from "pdfjs-dist/legacy/build/pdf.mjs";
import type { PdfRect as SourceRect } from "@research-assistant/core/pdf-view";
import { readPdfTextContent } from "./pdfTextContent";

export interface PdfPageText { readonly pageIndex: number; readonly text: string }

export function cleanupPdfDocument(document: PDFDocumentProxy | undefined, isCurrent: () => boolean, onError: (message: string) => void): void {
  if (!document) return;
  void document.cleanup().catch((reason: unknown) => {
    if (isCurrent()) onError(`释放 PDF 资源失败：${reason instanceof Error ? reason.message : String(reason)}`);
  });
}

/** Full-text indexing yields between pages and never fills the visible-page glyph cache. */
export async function extractPdfPageTexts(document: PDFDocumentProxy | undefined, isCurrent: () => boolean, signal?: AbortSignal): Promise<readonly PdfPageText[]> {
  if (!document) return [];
  const pages: PdfPageText[] = [];
  for (let pageIndex = 0; pageIndex < document.numPages; pageIndex += 1) {
    // A task boundary lets interactive page/thumbnail requests run even for cached text.
    await new Promise<void>((resolve) => window.setTimeout(resolve, 0));
    if (signalAborted(signal) || !isCurrent()) return [];
    const page = await document.getPage(pageIndex + 1);
    try {
      if (signalAborted(signal) || !isCurrent()) return [];
      const content = await readPdfTextContent(page);
      if (signalAborted(signal) || !isCurrent()) return [];
      pages.push({ pageIndex, text: content.items.map(part => "str" in part ? part.str : "").join(" ") });
    } finally { page.cleanup(); }
  }
  return pages;
}

/** Labels and outline cannot hold up each other or the visible-page renderer. */
export function loadPdfNavigationMetadata(document: PDFDocumentProxy, isCurrent: () => boolean, callbacks: {
  readonly labels: (labels: readonly string[]) => void;
  readonly outline: (outline: readonly PdfOutlineEntry[]) => void;
  readonly status: (message: string) => void;
}): void {
  void document.getPageLabels().then(labels => {
    if (isCurrent()) callbacks.labels(labels ?? []);
  }).catch((reason: unknown) => { if (isCurrent()) callbacks.status(`无法读取 PDF 页标签：${String(reason)}`); });
  void readPdfOutline(document).then(outline => {
    if (isCurrent()) callbacks.outline(outline);
  }).catch((reason: unknown) => { if (isCurrent()) callbacks.status(`无法读取 PDF 目录：${String(reason)}`); });
}

export async function renderPdfPageRegion(
    document: PDFDocumentProxy | undefined,
    isCurrent: () => boolean,
    pageIndex: number,
    rect: SourceRect,
    signal?: AbortSignal,
  ): Promise<string> {
    if (document === undefined || signalAborted(signal)) {
      throw new Error("PDF 尚未准备好，无法生成结构化图片截图");
    }
    const page = await document.getPage(pageIndex + 1);
    try {
      if (!isCurrent() || signalAborted(signal)) {
        throw new Error("PDF 阅读器状态已变化，截图已取消");
      }
      const viewport = page.getViewport({ scale: 2 });
      const pageCanvas = globalThis.document.createElement("canvas");
      const pageContext = pageCanvas.getContext("2d");
      if (pageContext === null) throw new Error("当前环境无法创建 PDF 截图画布");
      pageCanvas.width = Math.ceil(viewport.width);
      pageCanvas.height = Math.ceil(viewport.height);
      const task = page.render({ canvas: pageCanvas, canvasContext: pageContext, viewport });
      const cancel = (): void => task.cancel();
      signal?.addEventListener("abort", cancel, { once: true });
      try {
        await task.promise;
      } catch (reason: unknown) {
        if (signalAborted(signal)) throw new Error("结构化图片截图已取消");
        throw reason;
      } finally {
        signal?.removeEventListener("abort", cancel);
      }
      if (!isCurrent() || signalAborted(signal)) throw new Error("PDF 已变化，截图已取消");
      const x = Math.max(0, Math.floor(rect.x * pageCanvas.width));
      const y = Math.max(0, Math.floor(rect.y * pageCanvas.height));
      const width = Math.min(pageCanvas.width - x, Math.ceil(rect.width * pageCanvas.width));
      const height = Math.min(pageCanvas.height - y, Math.ceil(rect.height * pageCanvas.height));
      if (width <= 0 || height <= 0) {
        throw new Error("结构化图片区域为空，无法截图");
      }
      const cropCanvas = globalThis.document.createElement("canvas");
      const cropContext = cropCanvas.getContext("2d");
      if (cropContext === null) throw new Error("当前环境无法创建 PDF 截图画布");
      cropCanvas.width = width;
      cropCanvas.height = height;
      cropContext.drawImage(pageCanvas, x, y, width, height, 0, 0, width, height);
      return cropCanvas.toDataURL("image/png");
    } finally {
      page.cleanup();
    }
  }

export async function renderPdfThumbnail(
    document: PDFDocumentProxy | undefined,
    isCurrent: () => boolean,
    pageNumber: number,
    canvas: HTMLCanvasElement,
    signal?: AbortSignal,
  ): Promise<void> {
    if (document === undefined || signalAborted(signal)) return;
    const page = await document.getPage(pageNumber);
    try {
      if (!isCurrent() || signalAborted(signal)) return;
      const viewport = page.getViewport({ scale: 0.18 });
      const context = canvas.getContext("2d");
      if (context === null) return;
      canvas.width = Math.ceil(viewport.width);
      canvas.height = Math.ceil(viewport.height);
      const task = page.render({ canvas, canvasContext: context, viewport });
      const cancel = (): void => task.cancel();
      signal?.addEventListener("abort", cancel, { once: true });
      try {
        await task.promise;
      } catch (reason: unknown) {
        if (!signalAborted(signal)) throw reason;
      } finally {
        signal?.removeEventListener("abort", cancel);
      }
    } finally {
      page.cleanup();
    }
  }


function signalAborted(signal?: AbortSignal): boolean { return signal?.aborted === true; }
export async function readPdfOutline(document: PDFDocumentProxy): Promise<PdfOutlineEntry[]> {
    const entries = await document.getOutline();
    if (entries === null) return [];
    const outline: PdfOutlineEntry[] = [];
    for (const entry of entries) {
      const destination = typeof entry.dest === "string"
        ? await document.getDestination(entry.dest)
        : entry.dest;
      const reference = destination?.[0];
      if (typeof reference === "object" && reference !== null) {
        outline.push({ title: entry.title, pageNumber: (await document.getPageIndex(reference)) + 1 });
      }
    }
    return outline;
}
