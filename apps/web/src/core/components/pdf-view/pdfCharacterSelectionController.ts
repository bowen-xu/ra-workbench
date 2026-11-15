import {
  type PdfCharacterSelection,
  type PdfCharacterOffset,
  type PdfNormalizedRect,
  type PdfPageCharacterModel,
  type PdfPageCharacterStore,
} from "./pdfPageCharacterModel";
import {
  afterPdfCharacter,
  beforePdfCharacter,
  createPdfMultiPageCharacterSelection,
  createPdfLiveCharacterSegments,
  expandPdfCharacterRange,
  hitTestPdfCharacterOffset,
} from "./pdfPageCharacterSelection";

export interface PdfLiveCharacterSelection {
  readonly segments: readonly {
    readonly pageIndex: number;
    readonly rects: readonly PdfNormalizedRect[];
  }[];
}

export interface PdfCharacterSelectionControllerOptions {
  readonly viewer: HTMLElement;
  readonly doubleClickAction?: boolean;
  readonly pageStore: PdfPageCharacterStore;
  readonly onSelectionStart: () => void;
  readonly onSelectionChange: (selection: PdfLiveCharacterSelection | undefined) => void;
  readonly onSelectionComplete: (selection: PdfCharacterSelection) => void;
}

interface ActivePointerSelection {
  readonly pointerId: number;
  readonly textLayer: HTMLElement;
  readonly anchorModel: PdfPageCharacterModel;
  anchor: PdfCharacterOffset;
  readonly originX: number;
  readonly originY: number;
  focus: PdfCharacterOffset;
  focusModel: PdfPageCharacterModel;
  hasDragged: boolean;
  hasStarted: boolean;
  lockExpandedRange: boolean;
}

const MINIMUM_DRAG_DISTANCE = 2;

export class PdfCharacterSelectionController {
  private activeSelection: ActivePointerSelection | undefined;
  private liveFrame: number | undefined;
  private pendingPoint: Pick<PointerEvent, "clientX" | "clientY" | "target"> | undefined;
  private publishedSelection: PdfLiveCharacterSelection | undefined;
  private suppressPointerClick = false;
  private readonly options: PdfCharacterSelectionControllerOptions;

  constructor(options: PdfCharacterSelectionControllerOptions) {
    this.options = options;
    options.viewer.addEventListener("pointerdown", this.pointerDown, true);
    options.viewer.addEventListener("mousedown", this.mouseDown, true);
    options.viewer.addEventListener("click", this.click, true);
    window.document.addEventListener("pointermove", this.pointerMove, { capture: true, passive: false });
    window.document.addEventListener("pointerup", this.pointerUp, true);
    window.document.addEventListener("pointercancel", this.pointerCancel, true);
  }

  destroy(): void {
    this.options.viewer.removeEventListener("pointerdown", this.pointerDown, true);
    this.options.viewer.removeEventListener("mousedown", this.mouseDown, true);
    this.options.viewer.removeEventListener("click", this.click, true);
    window.document.removeEventListener("pointermove", this.pointerMove, true);
    window.document.removeEventListener("pointerup", this.pointerUp, true);
    window.document.removeEventListener("pointercancel", this.pointerCancel, true);
    if (this.liveFrame !== undefined) window.cancelAnimationFrame(this.liveFrame);
    this.liveFrame = undefined;
    this.pendingPoint = undefined;
    this.activeSelection?.textLayer.classList.remove("reader-custom-selecting");
    this.clearSelectionMarkers(this.activeSelection);
    this.activeSelection = undefined;
    this.options.onSelectionChange(undefined);
  }

  private publishSelection(active: ActivePointerSelection): void {
    const segments = createPdfLiveCharacterSegments(active.anchorModel, active.anchor, active.focusModel, active.focus);
    const previous = this.publishedSelection?.segments ?? [];
    if (segments.length === previous.length && segments.every((segment, index) => {
      const before = previous[index];
      return before?.pageIndex === segment.pageIndex && before.rects.length === segment.rects.length
        && segment.rects.every((rect, rectIndex) => {
          const old = before.rects[rectIndex];
          return old && rect.x === old.x && rect.y === old.y && rect.width === old.width && rect.height === old.height;
        });
    })) return;
    this.publishedSelection = segments.length ? { segments } : undefined;
    this.options.onSelectionChange(this.publishedSelection);
  }

  private endSelection(event: PointerEvent, shouldCommit: boolean): void {
    const active = this.activeSelection;
    if (active === undefined || event.pointerId !== active.pointerId) return;
    event.preventDefault();
    const pending = this.pendingPoint;
    this.pendingPoint = undefined;
    if (this.liveFrame !== undefined) {
      window.cancelAnimationFrame(this.liveFrame);
      this.liveFrame = undefined;
    }
    // WebKit may coalesce all intermediate pointermove events during a text
    // drag. The release coordinates still describe the completed gesture, so
    // do not degrade it to a click merely because no move event was observed.
    if (Math.hypot(event.clientX - active.originX, event.clientY - active.originY) >= MINIMUM_DRAG_DISTANCE) {
      active.hasDragged = true;
      active.lockExpandedRange = false;
    }
    if (active.hasDragged) this.startSelection(active);
    if (!active.lockExpandedRange) {
      let focus = this.selectionPoint(event);
      // A release outside the PDF must still include the latest in-page move
      // even when its animation frame has not run yet.
      if ((!focus || Math.abs(focus.model.pageIndex - active.anchorModel.pageIndex) > 1) && pending) {
        focus = this.selectionPoint(pending);
      }
      if (focus !== undefined && Math.abs(focus.model.pageIndex - active.anchorModel.pageIndex) <= 1) {
        active.focusModel = focus.model;
        active.focus = focus.point;
      }
    }
    const selection = shouldCommit && active.hasDragged
      ? createPdfMultiPageCharacterSelection(active.anchorModel, active.anchor, active.focusModel, active.focus) : undefined;
    active.textLayer.classList.remove("reader-custom-selecting");
    this.clearSelectionMarkers(active);
    this.activeSelection = undefined;
    this.suppressPointerClick = active.hasDragged;
    if (shouldCommit && selection !== undefined && selection.text.length > 0) {
      this.options.onSelectionComplete(selection);
    }
    this.options.onSelectionChange(undefined);
    this.publishedSelection = undefined;
  }

  private readonly pointerDown = (event: PointerEvent): void => {
    if (!event.isPrimary || event.button !== 0) return;
    this.suppressPointerClick = false;
    const target = event.target instanceof Element ? event.target : null;
    // Business overlays can opt into the same worker-glyph selection surface.
    // Their ordinary click remains available until a text gesture starts.
    const textSurface = target?.closest<HTMLElement>("[data-pdf-text-selection]");
    const control = target?.closest("a,button,input");
    if (control && control !== textSurface) return;
    if (textSurface && event.metaKey) return;
    if (this.options.doubleClickAction && event.detail >= 2) { event.preventDefault(); return; }
    const pageElement = target?.closest<HTMLElement>(".pdfViewer .page[data-page-number]");
    const textLayer = textSurface ? pageElement?.querySelector<HTMLElement>(".textLayer") : target?.closest<HTMLElement>(".textLayer");
    if (textLayer === null || textLayer === undefined || pageElement === null || pageElement === undefined) return;
    const model = this.options.pageStore.syncPage(pageElement);
    if (model === undefined) return;
    const anchor = hitTestPdfCharacterOffset(model, event.clientX, event.clientY);
    if (anchor === undefined) return;
    const granularity = event.detail >= 4 ? "all" : event.detail === 3 ? "line" : event.detail === 2 ? "word" : "character";
    const expanded = expandPdfCharacterRange(model, anchor.index, granularity);
    if (expanded === undefined) return;
    const expandedAnchor = beforePdfCharacter(expanded.start);
    const expandedFocus = afterPdfCharacter(expanded.end - 1);

    // Cancel native selection at mousedown for mice so the compatibility event
    // still supplies the browser's click count; pen/touch start at pointerdown.
    if (event.pointerType !== "mouse") event.preventDefault();
    this.options.viewer.closest<HTMLElement>(".pdf-viewer-container")?.focus({ preventScroll: true });
    window.getSelection()?.removeAllRanges();
    textLayer.classList.add("reader-custom-selecting");
    pageElement.dataset.customSelectionActive = "true";
    this.publishedSelection = undefined;
    this.activeSelection = {
      pointerId: event.pointerId,
      textLayer,
      anchorModel: model,
      anchor: granularity === "character" ? anchor : expandedAnchor,
      focus: granularity === "character" ? anchor : expandedFocus,
      focusModel: model,
      originX: event.clientX,
      originY: event.clientY,
      hasDragged: granularity !== "character",
      hasStarted: false,
      lockExpandedRange: granularity !== "character",
    };
    if (!textSurface || granularity !== "character") {
      this.startSelection(this.activeSelection);
      this.publishSelection(this.activeSelection);
    }
  };

  private startSelection(active: ActivePointerSelection): void {
    if (active.hasStarted) return;
    active.hasStarted = true;
    this.options.onSelectionStart();
  }

  private readonly mouseDown = (event: MouseEvent): void => {
    const active = this.activeSelection;
    // PointerEvent.detail is zero in Chromium/WebKit. MouseEvent.detail carries
    // the browser's click count, including its system double-click timing.
    if (!active || event.button !== 0) return;
    event.preventDefault();
    if (event.detail < 2 || this.options.doubleClickAction) return;
    const granularity = event.detail >= 4 ? "all" : event.detail === 3 ? "line" : "word";
    const expanded = expandPdfCharacterRange(active.anchorModel, active.anchor.index, granularity);
    if (!expanded) return;
    active.anchor = beforePdfCharacter(expanded.start);
    active.focus = afterPdfCharacter(expanded.end - 1);
    active.hasDragged = true;
    active.lockExpandedRange = true;
    this.startSelection(active);
    this.publishSelection(active);
  };

  private readonly click = (event: MouseEvent): void => {
    if (!this.suppressPointerClick || event.detail === 0) return;
    this.suppressPointerClick = false;
    // A drag/double-click must not reopen the saved annotation's toolbar and
    // replace the fresh text selection. Keyboard-generated clicks still work.
    event.preventDefault();
    event.stopPropagation();
  };

  private readonly pointerMove = (event: PointerEvent): void => {
    const active = this.activeSelection;
    if (active === undefined || event.pointerId !== active.pointerId) return;
    event.preventDefault();
    const distance = Math.hypot(event.clientX - active.originX, event.clientY - active.originY);
    if (distance >= MINIMUM_DRAG_DISTANCE) {
      active.hasDragged = true;
      active.lockExpandedRange = false;
    }
    if (!active.hasStarted && !active.hasDragged) return;
    if (active.lockExpandedRange) return;
    this.pendingPoint = { clientX: event.clientX, clientY: event.clientY, target: event.target };
    if (this.liveFrame !== undefined) return;
    this.liveFrame = window.requestAnimationFrame(() => {
      this.liveFrame = undefined;
      const point = this.pendingPoint;
      this.pendingPoint = undefined;
      if (!point || this.activeSelection !== active) return;
      this.startSelection(active);
      const focus = this.selectionPoint(point);
      if (!focus || Math.abs(focus.model.pageIndex - active.anchorModel.pageIndex) > 1) return;
      if (focus.point.index === active.focus.index && focus.point.offset === active.focus.offset
        && focus.model === active.focusModel) return;
      active.focus = focus.point;
      active.focusModel = focus.model;
      focus.model.pageElement.dataset.customSelectionActive = "true";
      focus.model.pageElement.querySelector(".textLayer")?.classList.add("reader-custom-selecting");
      this.publishSelection(active);
    });
  };

  private readonly pointerUp = (event: PointerEvent): void => this.endSelection(event, true);
  private readonly pointerCancel = (event: PointerEvent): void => this.endSelection(event, false);

  private selectionPoint(event: Pick<PointerEvent, "clientX" | "clientY" | "target">): {
    readonly model: PdfPageCharacterModel;
    readonly point: PdfCharacterOffset;
  } | undefined {
    const target = window.document.elementFromPoint(event.clientX, event.clientY)
      ?? (event.target instanceof Element ? event.target : null);
    const pageElement = target?.closest<HTMLElement>(".pdfViewer .page[data-page-number]");
    if (pageElement === null || pageElement === undefined) return undefined;
    const model = this.options.pageStore.syncPage(pageElement);
    if (model === undefined) return undefined;
    const point = hitTestPdfCharacterOffset(model, event.clientX, event.clientY);
    return point === undefined ? undefined : { model, point };
  }

  private clearSelectionMarkers(active: ActivePointerSelection | undefined): void {
    if (active === undefined) return;
    this.options.viewer.querySelectorAll<HTMLElement>("[data-custom-selection-active]").forEach((page) => {
      page.removeAttribute("data-custom-selection-active");
      page.querySelector(".textLayer")?.classList.remove("reader-custom-selecting");
    });
  }
}
