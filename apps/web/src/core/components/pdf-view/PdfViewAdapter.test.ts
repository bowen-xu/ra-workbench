// @vitest-environment happy-dom

import { describe, expect, it } from "vitest";

import { PDF_VIEW_SECURITY_POLICY } from "./PdfViewAdapter";
import { pdfWorkerCharacterCapability } from "./pdfTextContent";

describe("PdfViewAdapter contracts", () => {
  it("freezes the reader security policy at the adapter boundary", () => {
    expect(PDF_VIEW_SECURITY_POLICY).toEqual({
      enableXfa: false,
      dynamicCodeEvaluation: false,
      scriptingManager: false,
      pdfHistory: false,
      preferences: false,
    });
    expect(Object.isFrozen(PDF_VIEW_SECURITY_POLICY)).toBe(true);
  });

  it("detects a missing worker character patch instead of allowing a DOM fallback", () => {
    const content = (items: readonly unknown[]) => ({ items, styles: {}, lang: null }) as never;
    expect(pdfWorkerCharacterCapability(content([]))).toBe("not-required");
    expect(pdfWorkerCharacterCapability(content([{ str: "PDF", hasEOL: false }]))).toBe("missing");
    expect(pdfWorkerCharacterCapability(content([{
      str: "PDF",
      hasEOL: false,
      chars: [{ c: "P", rect: [0, 0, 1, 1], rotation: 0 }],
    }]))).toBe("available");
  });
});
