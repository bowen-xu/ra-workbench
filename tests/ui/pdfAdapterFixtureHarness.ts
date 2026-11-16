import type { PdfViewState } from "@research-assistant/core/pdf-view";
import type { ReaderViewStateV2 } from "@research-assistant/core/reader";
import type { Page } from "@playwright/test";

export interface PdfAdapterFixtureOptions {
  readonly restoredViewState?: ReaderViewStateV2;
  readonly navigatePageIndex?: number;
  readonly measureResources?: boolean;
}

export interface PdfAdapterResourceProbe {
  readonly suspended: boolean;
  readonly cachedPageIds: readonly number[];
  readonly canvasCount: number;
  readonly resourceApiAvailable: boolean;
}

export interface PdfCanvasProbe {
  readonly pageNumber: number;
  readonly bitmapWidth: number;
  readonly bitmapHeight: number;
  readonly cssWidth: number;
  readonly cssHeight: number;
  readonly intersectsViewport: boolean;
  readonly hasPaintedPixels: boolean;
}

export interface PdfAdapterFixtureResult {
  readonly pageCount: number;
  readonly pageHostCount: number;
  readonly readyViewState: PdfViewState;
  readonly pageChanges: readonly number[];
  readonly viewStates: readonly PdfViewState[];
  readonly canvases: readonly PdfCanvasProbe[];
  readonly active?: PdfAdapterResourceProbe;
  readonly trimmed?: PdfAdapterResourceProbe;
  readonly suspended?: PdfAdapterResourceProbe;
}

export async function openPdfAdapterFixture(
  page: Page,
  fixturePath: string,
  options: PdfAdapterFixtureOptions = {},
): Promise<PdfAdapterFixtureResult> {
  await page.route("**/__reader-fixtures/document.pdf", async (route) => {
    await route.fulfill({ path: fixturePath, contentType: "application/pdf" });
  });
  await page.goto("/");
  return page.evaluate(async (input) => {
    await import("/src/core/components/pdf-view/PdfView.css");
    const adapterPath = "/src/core/components/pdf-view/PdfViewAdapter.ts";
    const { PdfViewAdapter } = await import(adapterPath);
    const bytes = new Uint8Array(await fetch("/__reader-fixtures/document.pdf").then((response) => response.arrayBuffer()));
    const container = document.createElement("div");
    Object.assign(container.style, {
      position: "fixed",
      inset: "0",
      overflow: "auto",
      width: "900px",
      height: "700px",
    });
    const viewer = document.createElement("div");
    viewer.className = "pdfViewer";
    container.className = "pdf-view pdf-viewer-container";
    container.append(viewer);
    document.body.replaceChildren(container);

    type Snapshot = {
      readonly suspended: boolean;
      readonly cachedPageIds: readonly number[];
      readonly canvasCount: number;
      readonly resourceApiAvailable: boolean;
    };
    type ViewState = {
      readonly pageIndex: number;
      readonly scale: { readonly kind: string; readonly value?: number };
      readonly top?: number;
      readonly left?: number;
      readonly scrollMode: string;
      readonly spreadMode: string;
    };
    const snapshots: Snapshot[] = [];
    const pageChanges: number[] = [];
    const viewStates: ViewState[] = [];
    let resolveReady: ((event: { readonly pageCount: number; readonly viewState: ViewState }) => void) | undefined;
    const ready = new Promise<{ readonly pageCount: number; readonly viewState: ViewState }>((resolveEvent) => {
      resolveReady = resolveEvent;
    });
    const adapter = new PdfViewAdapter();
    await adapter.open({
      bytes,
      container,
      viewerElement: viewer,
      ...(input.restoredViewState === undefined ? {} : { initialPageIndex: input.restoredViewState.pageIndex }),
      callbacks: {
        onReady: (event: { readonly pageCount: number; readonly viewState: ViewState }) => resolveReady?.(event),
        onPageChange: (pageIndex: number) => pageChanges.push(pageIndex),
        onScaleChange: () => undefined,
        onViewAreaChange: (event: { readonly viewState: ViewState }) => viewStates.push(event.viewState),
        onPageHostsChange: () => undefined,
        onFindMatchesChange: () => undefined,
        onOutlineChange: () => undefined,
        onCharacterSelectionAvailabilityChange: () => undefined,
        onResourceChange: (snapshot: Snapshot) => snapshots.push(snapshot),
        onStatus: () => undefined,
      },
    });
    const readyEvent = await ready;
    await new Promise((resolveDelay) => window.setTimeout(resolveDelay, 1_500));
    if (input.navigatePageIndex !== undefined) {
      adapter.navigate({
        pageIndex: input.navigatePageIndex,
        scale: { kind: "page-fit" },
        history: "hard",
      }, input.navigatePageIndex);
      await new Promise((resolveDelay) => window.setTimeout(resolveDelay, 1_500));
    }
    const containerRect = container.getBoundingClientRect();
    const canvases = [...viewer.querySelectorAll<HTMLCanvasElement>(".page canvas")].map((canvas) => {
      const rect = canvas.getBoundingClientRect();
      const context = canvas.getContext("2d");
      let hasPaintedPixels = false;
      if (context !== null && canvas.width > 0 && canvas.height > 0) {
        const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
        const sampleStep = Math.max(1, Math.floor(Math.sqrt((canvas.width * canvas.height) / 12_000)));
        for (let y = 0; y < canvas.height && !hasPaintedPixels; y += sampleStep) {
          for (let x = 0; x < canvas.width; x += sampleStep) {
            const offset = (y * canvas.width + x) * 4;
            if ((pixels[offset + 3] ?? 0) > 0
              && ((pixels[offset] ?? 255) < 245 || (pixels[offset + 1] ?? 255) < 245 || (pixels[offset + 2] ?? 255) < 245)) {
              hasPaintedPixels = true;
              break;
            }
          }
        }
      }
      return {
        pageNumber: Number(canvas.closest<HTMLElement>(".page")?.dataset.pageNumber ?? 0),
        bitmapWidth: canvas.width,
        bitmapHeight: canvas.height,
        cssWidth: rect.width,
        cssHeight: rect.height,
        intersectsViewport: rect.right > containerRect.left
          && rect.left < containerRect.right
          && rect.bottom > containerRect.top
          && rect.top < containerRect.bottom,
        hasPaintedPixels,
      };
    });
    const active = snapshots.at(-1);
    const pageHostCount = viewer.querySelectorAll(".page[data-page-number]").length;
    let trimmed: Snapshot | undefined;
    let suspended: Snapshot | undefined;
    if (input.measureResources === true) {
      adapter.trimMemory();
      await new Promise((resolveDelay) => window.setTimeout(resolveDelay, 100));
      trimmed = snapshots.at(-1);
      adapter.suspend();
      await new Promise((resolveDelay) => window.setTimeout(resolveDelay, 100));
      suspended = snapshots.at(-1);
    }
    adapter.close();
    return {
      pageCount: readyEvent.pageCount,
      pageHostCount,
      readyViewState: readyEvent.viewState,
      pageChanges,
      viewStates,
      canvases,
      active,
      trimmed,
      suspended,
    };
  }, options) as Promise<PdfAdapterFixtureResult>;
}
