/** Public shared UI entry. Business data and session ownership remain in the consuming host. */
export { PdfView } from "../components/pdf-view/PdfView";
export { PdfViewAdapter } from "../components/pdf-view/PdfViewAdapter";
export { PdfViewController } from "../components/pdf-view/PdfViewController";
export type { PdfPageCharacterStore } from "../components/pdf-view/pdfPageCharacterModel";
export { PDF_BACKGROUND_SUSPEND_MS } from "../components/pdf-view/pdfResourcePolicy";
export type {
  PdfPageHost, PdfOutlineEntry, PdfViewReadyEvent, PdfViewAreaEvent,
  PdfResourceSnapshot, PdfFindOptions, PdfViewAdapterCallbacks, PdfViewOpenInput,
} from "../components/pdf-view/pdfViewTypes";
export { PdfPageControls, PdfViewControls } from "../components/pdf-view/PdfViewControls";
export { usePdfThumbnailCacheWindow } from "../components/usePdfThumbnailCacheWindow";
