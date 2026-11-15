// The distributed viewer expects pdfjsLib to be initialized before evaluation.
import "pdfjs-dist/legacy/build/pdf.mjs";
import type { PDFDocumentProxy, PDFPageProxy } from "pdfjs-dist/legacy/build/pdf.mjs";
import { PDFFindController } from "pdfjs-dist/legacy/web/pdf_viewer.mjs";
import { readPdfTextContent } from "./pdfTextContent";

/** PDF.js find uses getTextContent's async stream iterator, absent in some WKWebViews.
 * A find-only facade consumes the same public stream through getReader instead.
 * Rendering and the original document/page objects are never modified.
 */
export class PdfFindController extends PDFFindController {
  override setDocument(document: PDFDocumentProxy): void {
    // PDFViewer also calls this with null when releasing a document.
    if (!document) { super.setDocument(document); return; }
    const pages = new WeakMap<PDFPageProxy, PDFPageProxy>();
    const facade = new Proxy(document, {
      get(target, property) {
        if (property === "getPage") return async (pageNumber: number) => {
          const page = await target.getPage(pageNumber);
          let facadePage = pages.get(page);
          if (!facadePage) {
            facadePage = new Proxy(page, {
              get(original, key) {
                if (key === "getTextContent") return (options?: Parameters<PDFPageProxy["streamTextContent"]>[0]) => readPdfTextContent(original, options);
                return boundMember(original, key);
              },
            });
            pages.set(page, facadePage);
          }
          return facadePage;
        };
        return boundMember(target, property);
      },
    });
    super.setDocument(facade);
  }
}

function boundMember(target: object, key: string | symbol): unknown {
  const value: unknown = Reflect.get(target, key, target);
  return typeof value === "function" ? value.bind(target) : value;
}
