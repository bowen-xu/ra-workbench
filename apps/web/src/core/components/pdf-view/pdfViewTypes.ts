import type { PdfLocation, PdfViewState, PdfScale, PdfScaleLimits } from "@research-assistant/core/pdf-view";

export interface PdfPageHost {
  readonly pageIndex: number;
  readonly element: HTMLDivElement;
  readonly interactionElement?: HTMLDivElement;
}

export interface PdfOutlineEntry {
  readonly title: string;
  readonly pageNumber: number;
}

export interface PdfViewReadyEvent {
  readonly pageCount: number;
  readonly pageIndex: number;
  readonly pageLabels: readonly string[];
  readonly scale: number;
  readonly viewState: PdfViewState;
}

export interface PdfViewAreaEvent {
  readonly viewState: PdfViewState;
}

export interface PdfResourceSnapshot {
  readonly suspended: boolean;
  readonly cachedPageIds: readonly number[];
  readonly canvasCount: number;
  readonly resourceApiAvailable: boolean;
}

export interface PdfFindOptions {
  readonly query: string;
  readonly findPrevious: boolean;
  readonly type: "find" | "again";
  readonly caseSensitive: boolean;
  readonly entireWord: boolean;
  readonly highlightAll: boolean;
  readonly source?: object;
}

export type PdfDestinationLocation = Required<Pick<PdfLocation, "destination">>;

export interface PdfViewAdapterCallbacks {
  /** Document metadata is available independently of the first page's paint/setup. */
  readonly onDocumentReady?: (pageCount: number) => void;
  readonly onPageLabelsChange?: (labels: readonly string[]) => void;
  readonly onNavigate?: (location: PdfDestinationLocation) => void;
  readonly onReady: (event: PdfViewReadyEvent) => void;
  readonly onPageChange: (pageIndex: number) => void;
  readonly onScaleChange: (scale: number) => void;
  readonly onViewAreaChange: (event: PdfViewAreaEvent) => void;
  readonly onPageHostsChange: (hosts: readonly PdfPageHost[]) => void;
  readonly onFindMatchesChange: (current: number, total: number) => void;
  readonly onOutlineChange: (outline: readonly PdfOutlineEntry[]) => void;
  readonly onCharacterSelectionAvailabilityChange: (available: boolean) => void;
  readonly onResourceChange: (snapshot: PdfResourceSnapshot) => void;
  readonly onStatus: (status: string) => void;
}

export interface PdfViewOpenInput {
  readonly bytes: Uint8Array;
  readonly container: HTMLDivElement;
  readonly viewerElement: HTMLDivElement;
  readonly initialPageIndex?: number;
  readonly preserveView?: boolean;
  readonly initialView?: PdfViewState;
  readonly initialScale?: PdfScale;
  readonly scaleLimits?: PdfScaleLimits;
  readonly active?: boolean;
  readonly callbacks: PdfViewAdapterCallbacks;
}

export const PDF_VIEW_SECURITY_POLICY = Object.freeze({
  enableXfa: false,
  dynamicCodeEvaluation: false,
  scriptingManager: false,
  pdfHistory: false,
  preferences: false,
});

