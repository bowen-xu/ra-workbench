import { samePdfDocument, type PdfDocumentIdentity, type PdfSource, type PdfViewState, type PdfLocation, type PdfScale, type PdfScaleLimits, type PdfSelection } from "@research-assistant/core/pdf-view";
import { PdfViewAdapter, type PdfOutlineEntry, type PdfPageHost, type PdfFindOptions, type PdfResourceSnapshot, type PdfViewAdapterCallbacks } from "./PdfViewAdapter";
import { PDF_BACKGROUND_SUSPEND_MS } from "./pdfResourcePolicy";

export interface PdfViewSnapshot {
  readonly selection: PdfSelection | undefined;
  readonly selectionRevision: number;
  readonly scaleLimits: PdfScaleLimits;
  readonly ready: boolean;
  readonly loading: boolean;
  readonly source: PdfDocumentIdentity | undefined;
  readonly error: string | undefined;
  readonly status: string;
  readonly pageIndex: number;
  readonly pageCount: number;
  readonly pageLabels: readonly string[];
  readonly scalePercent: number;
  readonly viewState: PdfViewState | undefined;
  readonly pageHosts: readonly PdfPageHost[];
  readonly outline: readonly PdfOutlineEntry[];
  readonly findCurrent: number;
  readonly findTotal: number;
  readonly characterSelectionAvailable: boolean;
  readonly resources: PdfResourceSnapshot;
}
const initial = (): PdfViewSnapshot => ({
  selection: undefined, selectionRevision: 0, scaleLimits: { min: .5, max: 3 }, ready: false, loading: false, source: undefined, error: undefined, status: "",
  pageIndex: 0, pageCount: 0, pageLabels: [], scalePercent: 100, viewState: undefined,
  pageHosts: [], outline: [], findCurrent: 0, findTotal: 0, characterSelectionAvailable: false,
  resources: { suspended: false, cachedPageIds: [], canvasCount: 0, resourceApiAvailable: true },
});
export interface PdfViewOpenOptions {
  readonly source: PdfSource;
  readonly observer?: PdfViewAdapterCallbacks;
  readonly container: HTMLDivElement;
  readonly viewerElement: HTMLDivElement;
  readonly active: boolean;
  readonly initialPageIndex?: number;
  readonly initialView?: PdfViewState;
  readonly initialScale?: PdfScale;
  readonly scaleLimits?: PdfScaleLimits;
}

/** One document's rendering lifecycle. No persistence, notes or desktop services. */
export class PdfViewController {
  readonly adapter: PdfViewAdapter;
  #state = initial();
  #listeners = new Set<() => void>();
  #generation = 0;
  #displayGeneration = -1;
  #requestedKey: string | undefined;
  #active = true;
  #observer: PdfViewAdapterCallbacks | undefined;
  readonly #suspendDelayMs: number;
  #suspendTimer: number | undefined;
  constructor(adapter = new PdfViewAdapter(), suspendDelayMs = PDF_BACKGROUND_SUSPEND_MS) { this.adapter = adapter; this.#suspendDelayMs = suspendDelayMs; }
  readonly getSnapshot = (): PdfViewSnapshot => this.#state;
  readonly subscribe = (listener: () => void): (() => void) => { this.#listeners.add(listener); return () => this.#listeners.delete(listener); };
  #update(patch: Partial<PdfViewSnapshot>): void {
    this.#state = { ...this.#state, ...patch };
    this.#listeners.forEach((listener) => listener());
  }
  async open(options: PdfViewOpenOptions): Promise<void> {
    const sameKey = this.#state.source?.key === options.source.key;
    if (this.#requestedKey !== undefined && this.#requestedKey !== options.source.key) this.close();
    this.#requestedKey = options.source.key;
    const generation = ++this.#generation;
    this.#active = options.active;
    this.#observer = options.observer;
    const source = { key: options.source.key, revision: options.source.revision };
    const update = (patch: Partial<PdfViewSnapshot>): void => { if ((generation === this.#generation || generation === this.#displayGeneration)) this.#update(patch); };
    update({ loading: true, selection: undefined, selectionRevision: this.#state.selectionRevision + 1, error: undefined, scaleLimits: options.scaleLimits ?? { min: .5, max: 3 } });
    try {
      await this.adapter.open({
        bytes: options.source.bytes, container: options.container, viewerElement: options.viewerElement,
        active: options.active, preserveView: sameKey,
        ...(options.initialPageIndex === undefined ? {} : { initialPageIndex: options.initialPageIndex }),
        ...(options.initialView === undefined ? {} : { initialView: options.initialView }),
        ...(options.initialScale === undefined ? {} : { initialScale: options.initialScale }),
        ...(options.scaleLimits === undefined ? {} : { scaleLimits: options.scaleLimits }),
        callbacks: {
          onDocumentReady: (pageCount) => { if (generation !== this.#generation) return; update({ pageCount }); options.observer?.onDocumentReady?.(pageCount); },
          onPageLabelsChange: (pageLabels) => { if (generation !== this.#generation) return; update({ pageLabels }); options.observer?.onPageLabelsChange?.(pageLabels); },
          onReady: (event) => {
            if (generation !== this.#generation) return;
            this.#displayGeneration = generation;
            update({ ready: true, loading: false, source, pageIndex: event.pageIndex, pageCount: event.pageCount, pageLabels: event.pageLabels, scalePercent: Math.round(event.scale * 100), viewState: event.viewState, findCurrent: 0, findTotal: 0 });
            if (generation === this.#generation) { options.observer?.onReady(event); this.setActive(this.#active); }
          },
          onPageChange: (pageIndex) => { if (generation !== this.#generation && generation !== this.#displayGeneration) return; update({ pageIndex }); options.observer?.onPageChange(pageIndex); },
          onScaleChange: (scale) => { if (generation !== this.#generation && generation !== this.#displayGeneration) return; update({ scalePercent: Math.round(scale * 100) }); options.observer?.onScaleChange(scale); },
          onViewAreaChange: ({ viewState }) => { if (generation !== this.#generation && generation !== this.#displayGeneration) return; update({ viewState }); options.observer?.onViewAreaChange({ viewState }); },
          onNavigate: (location) => {
            if (generation !== this.#generation) return;
            if (options.observer?.onNavigate) options.observer.onNavigate(location);
            else this.navigate(location);
          },
          onPageHostsChange: (pageHosts) => { if (generation !== this.#generation && generation !== this.#displayGeneration) return; update({ pageHosts }); options.observer?.onPageHostsChange(pageHosts); },
          onFindMatchesChange: (findCurrent, findTotal) => { if (generation !== this.#generation && generation !== this.#displayGeneration) return; update({ findCurrent, findTotal }); options.observer?.onFindMatchesChange(findCurrent, findTotal); },
          onOutlineChange: (outline) => { if (generation !== this.#generation && generation !== this.#displayGeneration) return; update({ outline }); options.observer?.onOutlineChange(outline); },
          onCharacterSelectionAvailabilityChange: (characterSelectionAvailable) => { if (generation !== this.#generation && generation !== this.#displayGeneration) return; update({ characterSelectionAvailable }); options.observer?.onCharacterSelectionAvailabilityChange(characterSelectionAvailable); },
          onResourceChange: (resources) => { if (generation !== this.#generation && generation !== this.#displayGeneration) return; update({ resources }); options.observer?.onResourceChange(resources); },
          onStatus: (status) => { if (generation !== this.#generation && generation !== this.#displayGeneration) return; update({ status }); options.observer?.onStatus(status); },
        },
      });
    } catch (reason: unknown) {
      const error = reason instanceof Error ? reason.message : String(reason);
      update({ loading: false, error });
      if (generation === this.#generation) options.observer?.onStatus(error);
    }
  }
  setSelection(selection: PdfSelection | undefined): void { this.#update({ selection, selectionRevision: this.#state.selectionRevision + 1 }); }
  clearSelection(): void { this.setSelection(undefined); }
  async copySelection(): Promise<boolean> {
    const text = this.#state.selection?.text;
    if (!text) return false;
    await navigator.clipboard.writeText(text);
    return true;
  }
  isShowing(source: PdfDocumentIdentity): boolean { return this.#state.ready && !this.#state.loading && samePdfDocument(this.#state.source, source); }
  setActive(active: boolean): void {
    this.#active = active;
    if (this.#suspendTimer !== undefined) window.clearTimeout(this.#suspendTimer);
    this.#suspendTimer = undefined;
    this.adapter.setActive(active);
    if (!active) this.#suspendTimer = window.setTimeout(() => {
      if (this.#active) return;
      this.adapter.suspend();
      const resources = { ...this.#state.resources, suspended: true };
      this.#update({ resources }); this.#observer?.onResourceChange(resources);
    }, this.#suspendDelayMs);
  }
  navigate(location: PdfLocation): boolean {
    const labelIndex = location.pageLabel === undefined ? -1 : this.#state.pageLabels.indexOf(location.pageLabel);
    const index = location.pageIndex ?? location.position?.segments[0]?.pageIndex ?? (labelIndex < 0 ? undefined : labelIndex);
    const result = this.adapter.navigate(location, index);
    if (result) this.adapter.focus();
    return result;
  }
  setScale(scale: PdfScale): void { this.adapter.setScale(scale); }
  changeScale(delta: number): void { this.adapter.changeScale(delta); }
  updateScaleAt(factor: number, origin: readonly [number, number]): void { this.adapter.updateScaleAt(factor, origin); }
  find(options: PdfFindOptions): void { this.adapter.find(options); }
  closeFind(): void { this.adapter.closeFind(); }
  focus(): void { this.adapter.focus(); }
  close(): void {
    this.#generation += 1;
    this.#displayGeneration = -1;
    this.#requestedKey = undefined;
    if (this.#suspendTimer !== undefined) window.clearTimeout(this.#suspendTimer);
    this.#suspendTimer = undefined;
    this.adapter.close();
    this.#state = {...initial(), selectionRevision: this.#state.selectionRevision + 1};
    this.#listeners.forEach((listener) => listener());
  }
}
