import { describe, expect, it } from "vitest";

import {
  clampPdfPageIndex,
  mergePdfSelectionRects,
  parsePdfPageInput,
  pdfWheelZoomFactor,
  pdfWheelZoomFactorForInput,
  trimmedSelectionOffsets,
} from "./pdfReaderModel";

describe("PDF reader interaction model", () => {
  it("accepts page numbers and PDF page labels without allowing an invalid page", () => {
    expect(parsePdfPageInput("3", 8)).toBe(2);
    expect(parsePdfPageInput("iv", 8, ["i", "ii", "iii", "iv", "1", "2", "3", "4"])).toBe(3);
    expect(parsePdfPageInput("9", 8)).toBeUndefined();
    expect(clampPdfPageIndex(99, 8)).toBe(7);
  });

  it("turns trackpad and modified-wheel deltas into bounded zoom factors", () => {
    expect(pdfWheelZoomFactor(-4)).toBeGreaterThan(1);
    expect(pdfWheelZoomFactor(4)).toBeLessThan(1);
    expect(pdfWheelZoomFactor(-500)).toBe(1.15);
    expect(pdfWheelZoomFactor(500)).toBeCloseTo(1 / 1.15);
    expect(pdfWheelZoomFactor(0)).toBe(1);
  });

  it("keeps ordinary wheel input for scrolling and handles only modified zoom input", () => {
    expect(pdfWheelZoomFactorForInput({ deltaY: -4 })).toBeUndefined();
    expect(pdfWheelZoomFactorForInput({ deltaY: -4, ctrlKey: true })).toBeGreaterThan(1);
    expect(pdfWheelZoomFactorForInput({ deltaY: 4, metaKey: true })).toBeLessThan(1);
    expect(pdfWheelZoomFactorForInput({
      deltaY: -4,
      ctrlKey: true,
      nativeGestureActive: true,
    })).toBeUndefined();
  });

  it("excludes line-leading and line-trailing whitespace from PDF selection highlights", () => {
    expect(trimmedSelectionOffsets("  selected text   ")).toEqual({ start: 2, end: 15 });
    expect(trimmedSelectionOffsets("prefix selected suffix", 7, 16)).toEqual({ start: 7, end: 15 });
    expect(trimmedSelectionOffsets("   ")).toBeUndefined();
  });

  it("merges adjacent text fragments on one line without bridging PDF columns", () => {
    const rect = (left: number, top: number, width: number, height = 12) => ({
      left, top, width, height, right: left + width, bottom: top + height,
    });
    expect(mergePdfSelectionRects([
      rect(10, 20, 40),
      rect(51, 20, 30),
      rect(180, 20, 35),
      rect(10, 40, 60),
    ])).toEqual([
      rect(10, 20, 71),
      rect(180, 20, 35),
      rect(10, 40, 60),
    ]);
  });

  it("does not bridge columns when the right column has a slightly higher baseline", () => {
    const left = { left: 10, top: 20, right: 70, bottom: 32, width: 60, height: 12 };
    const right = { left: 180, top: 19, right: 240, bottom: 33, width: 60, height: 14 };
    expect(mergePdfSelectionRects([left, right])).toEqual([right, left]);
  });
});
