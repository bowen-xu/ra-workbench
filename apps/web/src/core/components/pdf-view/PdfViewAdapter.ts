import { selectPdfQuote } from "./pdfQuoteSelection";
import { selectPdfLines } from "./pdfLineSelection";
import type { SourceLineRange } from "@research-assistant/core/source";
import { bindPdfAutoLinks } from "./pdfAutoLinks";
import { PdfFindController } from "./PdfFindController";
import { ReaderPdfLinkService } from "./ReaderPdfLinkService";
import { pdfViewerZoomOrigin, setPdfViewerScale } from "./pdfViewerScale";
export { PDF_VIEW_SECURITY_POLICY } from "./pdfViewTypes";
import { PDF_VIEW_SECURITY_POLICY } from "./pdfViewTypes";
import { clientRectForPageRect, pdfPagePoint, pdfReadingPoint, loadPdfPageBox, normalizedPdfRect } from "./pdfCoordinates";
import { applyPdfLocation, clampPageIndex, pdfViewState, type PageChangingEvent, type ScaleChangingEvent, type TextLayerRenderedEvent, type ViewAreaEvent, type FindMatchesEvent, type PdfRenderingQueueRuntime, type PdfViewerRuntime, type PendingSecondNavigation } from "./pdfViewerRuntime";
import { cleanupPdfDocument, extractPdfPageTexts, loadPdfNavigationMetadata, renderPdfPageRegion, renderPdfThumbnail, type PdfPageText } from "./pdfDocumentRendering";
export type { PdfPageHost, PdfOutlineEntry, PdfViewReadyEvent, PdfViewAreaEvent, PdfResourceSnapshot, PdfFindOptions, PdfViewAdapterCallbacks, PdfViewOpenInput } from "./pdfViewTypes";
import type { PdfFindOptions, PdfViewAdapterCallbacks, PdfViewOpenInput } from "./pdfViewTypes";
import {
  GlobalWorkerOptions,
  getDocument,
  type PDFDocumentLoadingTask,
  type PDFDocumentProxy,
} from "pdfjs-dist/legacy/build/pdf.mjs";
import {
  EventBus,
  PDFLinkService,
  PDFViewer,
  ScrollMode,
} from "pdfjs-dist/legacy/web/pdf_viewer.mjs";
import workerUrl from "pdfjs-dist/legacy/build/pdf.worker.mjs?url";
import { clampPdfScale, type PdfScaleLimits, type PdfLocation, type PdfScale, type PdfViewState, type PdfPagePoint } from "@research-assistant/core/pdf-view";
type SourceRect = import("@research-assistant/core/pdf-view").PdfRect;
import { PdfPageCharacterStore } from "./pdfPageCharacterModel";
import {
  pdfSourceCharacters,
  pdfWorkerCharacterCapability,
  readPdfTextContent,
} from "./pdfTextContent";
import { PdfThumbnailQueue } from "./PdfThumbnailQueue";
import {
  PDF_RECENT_PAGE_BUFFER,
  supportsPdfViewerResourceApi,
  trimPdfPageBuffer,
  type PdfViewerResourceApi,
} from "./pdfResourcePolicy";
GlobalWorkerOptions.workerSrc = workerUrl;

export class PdfViewAdapter {
  readonly characterStore = new PdfPageCharacterStore();
  #loadingTask: PDFDocumentLoadingTask | undefined;
  #pendingTask: PDFDocumentLoadingTask | undefined;
  #request = 0;
  #limits: PdfScaleLimits = { min: .5, max: 3 };
  #viewState: PdfViewState | undefined;
  #document: PDFDocumentProxy | undefined;
  #viewer: PDFViewer | undefined;
  #viewerAbort: AbortController | undefined;
  #eventBus: EventBus | undefined;
  #linkService: ReaderPdfLinkService | undefined;
  #container: HTMLDivElement | undefined;
  #viewerElement: HTMLDivElement | undefined;
  #callbacks: PdfViewAdapterCallbacks | undefined;
  #pageLabels: readonly string[] = [];
  #pageHostFrame: number | undefined;
  #secondNavigationTimer: number | undefined;
  #pendingSecondNavigation: PendingSecondNavigation | undefined;
  #generation = 0;
  #active = true;
  #suspended = false;
  #resourceApiWarningSent = false;
  #retainedPageIds: readonly number[] = [];
  #unbindViewer: (() => void) | undefined;
  #pendingCharacterPages = new Map<number, Promise<void>>();
  #thumbnailQueue = this.#createThumbnailQueue();
  #renderHighestPriority: PdfRenderingQueueRuntime["renderHighestPriority"] | undefined;
  #renderQueueHadOwnMethod = false;

  get pageCount(): number {
    return this.#document?.numPages ?? 0;
  }

  get ready(): boolean {
    return this.#viewer !== undefined && this.#document !== undefined;
  }

  async open(input: PdfViewOpenInput): Promise<void> {
    const request = ++this.#request;
    void this.#pendingTask?.destroy();
    const task = getDocument({ data: input.bytes.slice(), enableXfa: PDF_VIEW_SECURITY_POLICY.enableXfa });
    this.#pendingTask = task;
    input.callbacks.onStatus("正在载入 PDF…");
    try {
      const document = await task.promise;
      if (request !== this.#request) return;
      this.#pendingTask = undefined;
      const initialView = input.preserveView ? this.#viewState : input.initialView;
      this.#dispose();
      this.#limits = input.scaleLimits ?? { min: .5, max: 3 };
      this.#container = input.container;
      this.#viewerElement = input.viewerElement;
      this.#callbacks = input.callbacks;
      this.#active = input.active ?? true;
      this.#suspended = false;
      this.#resourceApiWarningSent = false;
      this.#retainedPageIds = [];
      this.#thumbnailQueue = this.#createThumbnailQueue();
      this.#loadingTask = task;
      this.#document = document;
      input.callbacks.onCharacterSelectionAvailabilityChange(true);
      input.callbacks.onStatus(`${document.numPages} 页`);
      input.callbacks.onDocumentReady?.(document.numPages);
      this.#createViewer(document, { ...input, ...(initialView ? { initialView } : {}) });
      loadPdfNavigationMetadata(document, () => document === this.#document, {
        labels: labels => {
          this.#pageLabels = labels;
          this.#viewer?.setPageLabels(labels.length > 0 ? [...labels] : null);
          this.#callbacks?.onPageLabelsChange?.(labels);
        },
        outline: outline => this.#callbacks?.onOutlineChange(outline),
        status: message => this.#callbacks?.onStatus(message),
      });
    } catch (reason: unknown) {
      if (request !== this.#request) return;
      this.#pendingTask = undefined;
      void task.destroy();
      throw reason;
    }
  }

  close(): void {
    this.#request += 1;
    void this.#pendingTask?.destroy();
    this.#pendingTask = undefined;
    this.#dispose();
  }

  #dispose(): void {
    this.#generation += 1;
    this.#unbindViewer?.();
    this.#unbindViewer = undefined;
    if (this.#pageHostFrame !== undefined) window.cancelAnimationFrame(this.#pageHostFrame);
    if (this.#secondNavigationTimer !== undefined) window.clearTimeout(this.#secondNavigationTimer);
    this.#pageHostFrame = undefined;
    this.#secondNavigationTimer = undefined;
    this.#pendingSecondNavigation = undefined;
    this.#restoreRenderingQueue();
    // PDF.js accepts null to cancel/reset; 6.1.200's declaration omits that runtime case.
    (this.#viewer as unknown as { setDocument(document: null): void } | undefined)?.setDocument(null);
    this.#viewerAbort?.abort();
    this.#viewerAbort = undefined;
    this.#viewerElement?.replaceChildren();
    this.#viewer = undefined;
    this.#eventBus = undefined;
    this.#linkService = undefined;
    this.#container = undefined;
    this.#viewerElement = undefined;
    this.#callbacks?.onPageHostsChange([]);
    this.#callbacks = undefined;
    this.#pageLabels = [];
    this.#pendingCharacterPages.clear();
    this.#viewState = undefined;
    this.#retainedPageIds = [];
    this.#thumbnailQueue.dispose();
    this.#suspended = false;
    this.characterStore.clear();
    const loadingTask = this.#loadingTask;
    const document = this.#document;
    this.#loadingTask = undefined;
    this.#document = undefined;
    if (loadingTask !== undefined) void loadingTask.destroy();
    else if (document !== undefined) void document.cleanup();
  }

  setActive(active: boolean): void {
    this.#active = active;
    if (active) {
      this.#thumbnailQueue.resume();
      this.resume();
      this.#completeSecondNavigation();
    } else {
      this.#thumbnailQueue.pause();
      this.trimMemory();
    }
  }

  trimMemory(): void {
    this.#trimPageBuffer(0);
    this.#viewer?.cleanup();
    // Do not clean document fonts/operators while retained pages or a cancelled
    // thumbnail are still painting. Document-wide cleanup belongs to suspension.
    this.#refreshPageHosts();
    this.#emitResourceSnapshot();
  }

  suspend(): void {
    if (this.#suspended) return;
    const viewer = this.#viewer as unknown as PdfViewerRuntime | undefined;
    if (viewer === undefined) return;
    this.#suspended = true;
    const queue = viewer.renderingQueue;
    this.#renderQueueHadOwnMethod = Object.prototype.hasOwnProperty.call(queue, "renderHighestPriority");
    this.#renderHighestPriority = queue.renderHighestPriority;
    queue.renderHighestPriority = (): undefined => undefined;
    for (let pageIndex = 0; pageIndex < viewer.pagesCount; pageIndex += 1) {
      const pageView = viewer.getPageView(pageIndex);
      pageView?.destroy();
      this.characterStore.releasePage(pageIndex);
    }
    this.#retainedPageIds = [];
    this.#thumbnailQueue.pause();
    const document = this.#document;
    void this.#thumbnailQueue.whenIdle().then(() => {
      if (this.#suspended && document === this.#document) cleanupPdfDocument(document, () => document === this.#document, message => this.#callbacks?.onStatus(message));
    });
    this.#refreshPageHosts();
    this.#emitResourceSnapshot();
  }

  resume(): void {
    if (!this.#suspended) {
      if (this.#active) this.#viewer?.update();
      return;
    }
    this.#restoreRenderingQueue();
    this.#suspended = false;
    if (this.#active) {
      this.#thumbnailQueue.resume();
      this.#viewer?.update();
    }
    this.#emitResourceSnapshot();
  }

  navigate(location: PdfLocation, resolvedPageIndex?: number): boolean {
    if (location.destination !== undefined) {
      const linkService = this.#linkService;
      if (linkService === undefined) return false;
      void linkService.navigateToDestination(location.destination as Parameters<PDFLinkService["goToDestination"]>[0])
        .catch((reason: unknown) => {
          this.#callbacks?.onStatus(`无法打开 PDF 目标：${reason instanceof Error ? reason.message : String(reason)}`);
        });
      return true;
    }
    const viewer = this.#viewer;
    if (viewer === undefined || resolvedPageIndex === undefined || this.pageCount === 0) return false;
    const pageIndex = clampPageIndex(resolvedPageIndex, this.pageCount);
    if (location.ifNeeded === true && (location.visibilityMargin ?? 0) === 0 && supportsPdfViewerResourceApi(viewer)) {
      const resourceViewer = viewer as unknown as PdfViewerResourceApi;
      if (resourceViewer._getVisiblePages().ids.has(pageIndex + 1)) return true;
    }
    this.#applyLocation(location, pageIndex);
    if (location.position !== undefined || location.top !== undefined || location.left !== undefined) this.#scheduleSecondNavigation(location, pageIndex);
    return true;
  }

  async resolveQuoteSelection(pageIndex: number, quote: string, prefix: string, suffix: string) {
    const generation = this.#generation;
    await this.#ensurePageCharacters(pageIndex);
    if (generation !== this.#generation) return undefined;
    const page = this.#viewer?.getPageView(pageIndex)?.div;
    if (page === undefined) return undefined;
    const model = this.characterStore.syncPage(page);
    return model === undefined ? undefined : selectPdfQuote(model, quote, prefix, suffix);
  }
  async resolveLineSelection(pageIndex: number, range: SourceLineRange) {
    const generation = this.#generation;
    await this.#ensurePageCharacters(pageIndex);
    if (generation !== this.#generation) return undefined;
    const page = this.#viewer?.getPageView(pageIndex)?.div;
    if (!page) return undefined;
    const model = this.characterStore.syncPage(page);
    return model ? selectPdfLines(model, range) : undefined;
  }

  getViewState(): PdfViewState | undefined { return this.#viewState; }

  setScale(scale: PdfScale): void {
    if (this.#viewer) setPdfViewerScale(this.#viewer, scale, this.#limits);
  }

  pagePoint(x: number, y: number): PdfPagePoint | undefined { return pdfPagePoint(this.#viewerElement, this.#viewer, x, y); }
  readingPoint(offsetTopCssPx = 72): PdfPagePoint | undefined { return pdfReadingPoint(this.#container, this.#viewerElement, this.#viewer, offsetTopCssPx); }
  loadPageBox(pageIndex: number): Promise<readonly [number, number, number, number] | undefined> {
    const document = this.#document;
    return loadPdfPageBox(document, () => document === this.#document, pageIndex);
  }
  normalizedPdfRect(pageIndex: number, rect: SourceRect): Promise<SourceRect | undefined> {
    const document = this.#document;
    return normalizedPdfRect(document, () => document === this.#document, pageIndex, rect);
  }

  focus(): void {
    this.#container?.focus({ preventScroll: true });
  }

  clientRectForPageRect(pageIndex: number, rect: SourceRect): DOMRect | undefined {
    return clientRectForPageRect(this.#viewerElement, pageIndex, rect);
  }

  changeScale(difference: number, minimum = this.#limits.min, maximum = this.#limits.max): void {
    const viewer = this.#viewer;
    if (viewer === undefined) return;
    viewer.currentScaleValue = String(Math.max(minimum, Math.min(maximum, viewer.currentScale + difference)));
  }

  fitToWidth(): void {
    if (this.#viewer !== undefined) setPdfViewerScale(this.#viewer, { kind: "page-width" }, this.#limits);
  }

  updateScaleAt(scaleFactor: number, origin: readonly [number, number]): void {
    const viewer = this.#viewer;
    if (viewer === undefined || !Number.isFinite(scaleFactor) || scaleFactor <= 0) return;
    const targetScale = clampPdfScale(viewer.currentScale * scaleFactor, this.#limits);
    if (targetScale === viewer.currentScale) return;
    viewer.updateScale({
      scaleFactor: targetScale / viewer.currentScale,
      drawingDelay: 220,
      origin: pdfViewerZoomOrigin(viewer, origin),
    });
  }

  find(options: PdfFindOptions): void {
    this.#eventBus?.dispatch("find", {
      source: options.source ?? this,
      type: options.type,
      query: options.query,
      phraseSearch: true,
      caseSensitive: options.caseSensitive,
      entireWord: options.entireWord,
      highlightAll: options.highlightAll,
      findPrevious: options.findPrevious,
      matchDiacritics: false,
    });
  }

  closeFind(source?: object): void {
    this.#eventBus?.dispatch("findbarclose", { source: source ?? this });
  }

  async extractPageTexts(signal?: AbortSignal): Promise<readonly PdfPageText[]> {
    const document = this.#document;
    return extractPdfPageTexts(document, () => document === this.#document, signal);
  }

  async renderThumbnail(pageNumber: number, canvas: HTMLCanvasElement, signal?: AbortSignal): Promise<void> {
    return this.#thumbnailQueue.enqueue({
      pageNumber, canvas,
      ...(signal === undefined ? {} : { signal }),
    });
  }

  renderPageRegionPng(pageIndex: number, rect: SourceRect, signal?: AbortSignal): Promise<string> {
    const document = this.#document;
    return renderPdfPageRegion(document, () => document === this.#document, pageIndex, rect, signal);
  }

  #createViewer(document: PDFDocumentProxy, input: PdfViewOpenInput): void {
    const eventBus = new EventBus();
    const linkService = new ReaderPdfLinkService({ eventBus, ignoreDestinationZoom: true }, input.callbacks.onNavigate);
    const findController = new PdfFindController({ eventBus, linkService });
    this.#viewerAbort = new AbortController();
    const viewerOptions = {
      abortSignal: this.#viewerAbort.signal,
      container: input.container,
      viewer: input.viewerElement,
      eventBus,
      linkService,
      findController,
      removePageBorders: false,
      enableDetailCanvas: true,
      enableAutoLinking: false,
    };
    const viewer = new PDFViewer(viewerOptions);
    this.#eventBus = eventBus;
    this.#linkService = linkService;
    this.#viewer = viewer;
    linkService.setViewer(viewer);
    linkService.setDocument(document);
    findController.setDocument(document);

    const pagesInitialized = (): void => {
      viewer.scrollMode = ScrollMode.VERTICAL;
      const target = clampPageIndex(input.initialView?.pageIndex ?? input.initialPageIndex ?? 0, document.numPages);
      // Repeat the requested view after actual page dimensions settle.
      const initialLocation: PdfLocation = {
        ...input.initialView,
        pageIndex: target,
        scale: input.initialView?.scale ?? input.initialScale ?? { kind: "page-fit" },
      };
      this.#applyLocation(initialLocation, target);
      this.#scheduleSecondNavigation(initialLocation, target);
      input.callbacks.onReady({
        pageCount: document.numPages,
        pageIndex: target,
        pageLabels: this.#pageLabels,
        scale: viewer.currentScale,
        viewState: {
          pageIndex: target,
          scale: initialLocation.scale ?? { kind: "page-fit" },
          ...(initialLocation.top === undefined ? {} : { top: initialLocation.top }),
          ...(initialLocation.left === undefined ? {} : { left: initialLocation.left }),
          scrollMode: "vertical",
          spreadMode: "none",
        },
      });
      this.#refreshPageHosts();
    };
    const pageChanged = ({ pageNumber }: PageChangingEvent): void => {
      input.callbacks.onPageChange(pageNumber - 1);
    };
    const scaleChanged = ({ scale }: ScaleChangingEvent): void => {
      input.callbacks.onScaleChange(scale);
    };
    const viewAreaChanged = ({ location }: ViewAreaEvent): void => {
      this.#viewState = pdfViewState(viewer, location, this.#limits);
      input.callbacks.onViewAreaChange({ viewState: this.#viewState });
      this.#trimPageBuffer(PDF_RECENT_PAGE_BUFFER);
    };
    const textLayerRendered = ({ pageNumber }: TextLayerRenderedEvent): void => {
      const pageIndex = pageNumber - 1;
      this.characterStore.invalidate(pageIndex);
      void this.#ensurePageCharacters(pageIndex).catch((reason: unknown) => {
        if (this.#viewer === viewer) {
          input.callbacks.onStatus(`读取文字坐标失败：${reason instanceof Error ? reason.message : String(reason)}`);
        }
      });
      this.#refreshPageHosts();
    };
    const findMatchesChanged = ({ matchesCount }: FindMatchesEvent): void => {
      input.callbacks.onFindMatchesChange(matchesCount?.current ?? 0, matchesCount?.total ?? 0);
    };

    eventBus.on("pagesinit", pagesInitialized);
    eventBus.on("pagechanging", pageChanged);
    eventBus.on("scalechanging", scaleChanged);
    eventBus.on("updateviewarea", viewAreaChanged);
    eventBus.on("pagerendered", this.#pageRendered);
    eventBus.on("textlayerrendered", textLayerRendered);
    eventBus.on("updatefindmatchescount", findMatchesChanged);
    const unbindAutoLinks = bindPdfAutoLinks(eventBus, viewer, this.characterStore,
      (pageIndex) => this.#ensurePageCharacters(pageIndex), () => this.#viewer === viewer,
      (reason) => { if (this.#viewer === viewer) input.callbacks.onStatus(`自动链接生成失败：${String(reason)}`); });
    this.#unbindViewer = () => {
      unbindAutoLinks();
      eventBus.off("pagesinit", pagesInitialized);
      eventBus.off("pagechanging", pageChanged);
      eventBus.off("scalechanging", scaleChanged);
      eventBus.off("updateviewarea", viewAreaChanged);
      eventBus.off("pagerendered", this.#pageRendered);
      eventBus.off("textlayerrendered", textLayerRendered);
      eventBus.off("updatefindmatchescount", findMatchesChanged);
    };
    viewer.setDocument(document);
    if (!this.#active) this.setActive(false);
  }

  readonly #pageRendered = (event?: { readonly pageNumber?: number }): void => {
    const pending = this.#pendingSecondNavigation;
    if (pending !== undefined && event?.pageNumber === pending.pageIndex + 1) {
      this.#completeSecondNavigation();
    }
    this.#trimPageBuffer(PDF_RECENT_PAGE_BUFFER);
    this.#refreshPageHosts();
  };

  readonly #refreshPageHosts = (): void => {
    if (this.#pageHostFrame !== undefined) return;
    this.#pageHostFrame = window.requestAnimationFrame(() => {
      this.#pageHostFrame = undefined;
      const viewerElement = this.#viewerElement;
      if (viewerElement === undefined) return;
      const hosts = [...viewerElement.querySelectorAll<HTMLElement>(".page[data-page-number]")]
        .map((page) => {
          this.characterStore.syncPage(page);
          let layer = page.querySelector<HTMLDivElement>(":scope > .pdf-view-overlay-layer");
          if (layer === null) {
            layer = window.document.createElement("div");
            layer.className = "pdf-view-overlay-layer";
            page.append(layer);
          }
          let interactionLayer = page.querySelector<HTMLDivElement>(":scope > .pdf-view-interaction-layer");
          if (interactionLayer === null) {
            interactionLayer = window.document.createElement("div");
            interactionLayer.className = "pdf-view-interaction-layer";
            page.append(interactionLayer);
          }
          return { pageIndex: Number(page.dataset.pageNumber) - 1, element: layer, interactionElement: interactionLayer };
        });
      this.#callbacks?.onPageHostsChange(hosts);
    });
  };

  #ensurePageCharacters(pageIndex: number): Promise<void> {
    if (this.characterStore.hasPageCharacters(pageIndex)) return Promise.resolve();
    const pending = this.#pendingCharacterPages.get(pageIndex);
    if (pending !== undefined) return pending;
    const document = this.#document;
    if (document === undefined) return Promise.resolve();
    const generation = this.#generation;
    const load = document.getPage(pageIndex + 1)
      .then(async (page) => {
        try {
          const content = await readPdfTextContent(page);
          if (generation !== this.#generation || document !== this.#document) return;
          this.#acceptPageCharacters(pageIndex, content, page.getViewport({ scale: 1 }));
          this.#refreshPageHosts();
        } finally {
          page.cleanup();
        }
      })
      .finally(() => { if (generation === this.#generation) this.#pendingCharacterPages.delete(pageIndex); });
    this.#pendingCharacterPages.set(pageIndex, load);
    return load;
  }

  #acceptPageCharacters(
    pageIndex: number,
    content: Parameters<typeof pdfWorkerCharacterCapability>[0],
    viewport: Parameters<typeof pdfSourceCharacters>[1],
  ): void {
    const capability = pdfWorkerCharacterCapability(content);
    if (capability === "missing") {
      this.#callbacks?.onCharacterSelectionAvailabilityChange(false);
      return;
    }
    if (capability === "available") {
      this.characterStore.setPageCharacters(pageIndex, pdfSourceCharacters(content, viewport));
      this.#callbacks?.onCharacterSelectionAvailabilityChange(true);
    }
  }

  #createThumbnailQueue(): PdfThumbnailQueue {
    return new PdfThumbnailQueue((job) => {
      const document = this.#document;
      return renderPdfThumbnail(document, () => document === this.#document, job.pageNumber, job.canvas, job.signal);
    });
  }

  #applyLocation(location: PdfLocation, pageIndex: number): void {
    applyPdfLocation(this.#viewer, this.#container, this.#limits, location, pageIndex);
  }

  #scheduleSecondNavigation(location: PdfLocation, pageIndex: number): void {
    if (this.#secondNavigationTimer !== undefined) window.clearTimeout(this.#secondNavigationTimer);
    this.#pendingSecondNavigation = { location, pageIndex };
    this.#secondNavigationTimer = window.setTimeout(() => this.#completeSecondNavigation(), 10_000);
  }

  #completeSecondNavigation(): void {
    const pending = this.#pendingSecondNavigation;
    if (pending === undefined) return;
    if (!this.#active) {
      this.#secondNavigationTimer = undefined;
      return;
    }
    if (this.#secondNavigationTimer !== undefined) window.clearTimeout(this.#secondNavigationTimer);
    this.#secondNavigationTimer = undefined;
    this.#pendingSecondNavigation = undefined;
    this.#applyLocation(pending.location, pending.pageIndex);
  }

  #trimPageBuffer(recentPageCount: number): void {
    const viewer = this.#viewer;
    if (viewer === undefined) return;
    if (!supportsPdfViewerResourceApi(viewer)) {
      this.#retainedPageIds = [];
      if (!this.#resourceApiWarningSent) {
        this.#resourceApiWarningSent = true;
        this.#callbacks?.onStatus("PDF.js 页面缓存接口不可用；已停用主动页面裁剪");
      }
      this.#emitResourceSnapshot();
      return;
    }
    const result = trimPdfPageBuffer(viewer, recentPageCount, (pageId) => {
      this.characterStore.releasePage(pageId - 1);
    });
    this.#retainedPageIds = result.retainedPageIds;
    this.#emitResourceSnapshot();
  }

  #restoreRenderingQueue(): void {
    const viewer = this.#viewer as unknown as PdfViewerRuntime | undefined;
    const queue = viewer?.renderingQueue;
    const original = this.#renderHighestPriority;
    if (queue !== undefined && original !== undefined) {
      if (this.#renderQueueHadOwnMethod) queue.renderHighestPriority = original;
      else delete (queue as Partial<PdfRenderingQueueRuntime>).renderHighestPriority;
    }
    this.#renderHighestPriority = undefined;
    this.#renderQueueHadOwnMethod = false;
  }

  #emitResourceSnapshot(): void {
    const viewer = this.#viewer;
    const resourceApiAvailable = supportsPdfViewerResourceApi(viewer);
    this.#callbacks?.onResourceChange({
      suspended: this.#suspended,
      cachedPageIds: resourceApiAvailable ? this.#retainedPageIds : [],
      canvasCount: this.#viewerElement?.querySelectorAll("canvas").length ?? 0,
      resourceApiAvailable,
    });
  }

}

