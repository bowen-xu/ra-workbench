import { PDFLinkService } from "pdfjs-dist/legacy/web/pdf_viewer.mjs";
import type { PdfDestinationLocation } from "./pdfViewTypes";

type Destination = Parameters<PDFLinkService["goToDestination"]>[0];

/** Internal PDF links enter the host's navigation history, not browser history. */
export class ReaderPdfLinkService extends PDFLinkService {
  constructor(options: ConstructorParameters<typeof PDFLinkService>[0], private readonly onNavigate?: (location: PdfDestinationLocation) => void) {
    super(options);
  }

  override async goToDestination(destination: Destination): Promise<void> {
    if (this.onNavigate) this.onNavigate({ destination });
    else await super.goToDestination(destination);
  }

  /** The host already recorded this jump; resolve it without routing it again. */
  navigateToDestination(destination: Destination): Promise<void> {
    return super.goToDestination(destination);
  }
}
