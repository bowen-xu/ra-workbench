export interface PdfWheelZoomInput {
  readonly deltaY: number;
  readonly deltaMode?: number;
  readonly ctrlKey?: boolean;
  readonly metaKey?: boolean;
  readonly nativeGestureActive?: boolean;
}

export interface PdfSelectionRect {
  readonly left: number;
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
  readonly width: number;
  readonly height: number;
}

export function trimmedSelectionOffsets(
  text: string,
  startOffset = 0,
  endOffset = text.length,
): { readonly start: number; readonly end: number } | undefined {
  const selected = text.slice(startOffset, endOffset);
  const firstVisible = selected.search(/\S/u);
  if (firstVisible < 0) return undefined;
  const trailingWhitespace = selected.match(/\s*$/u)?.[0].length ?? 0;
  return {
    start: startOffset + firstVisible,
    end: endOffset - trailingWhitespace,
  };
}

export function mergePdfSelectionRects(
  input: readonly PdfSelectionRect[],
): readonly PdfSelectionRect[] {
  const sorted = [...input]
    .filter((rect) => rect.width > 0 && rect.height > 0)
    .sort((left, right) => left.top - right.top || left.left - right.left);
  const merged: PdfSelectionRect[] = [];
  for (const rect of sorted) {
    const previous = merged.at(-1);
    const sameLine = previous !== undefined
      && Math.min(previous.bottom, rect.bottom) - Math.max(previous.top, rect.top)
        >= Math.min(previous.height, rect.height) * 0.6;
    const inlineGap = previous === undefined
      ? Number.POSITIVE_INFINITY
      : Math.max(0, rect.left - previous.right, previous.left - rect.right);
    const nearby = previous !== undefined && inlineGap <= Math.max(previous.height, rect.height) * 1.5;
    if (!sameLine || !nearby) {
      merged.push(rect);
      continue;
    }
    const left = Math.min(previous.left, rect.left);
    const top = Math.min(previous.top, rect.top);
    const right = Math.max(previous.right, rect.right);
    const bottom = Math.max(previous.bottom, rect.bottom);
    merged[merged.length - 1] = { left, top, right, bottom, width: right - left, height: bottom - top };
  }
  return merged;
}

export function clampPdfPageIndex(pageIndex: number, pageCount: number): number {
  if (!Number.isFinite(pageIndex) || pageCount <= 0) return 0;
  return Math.max(0, Math.min(pageCount - 1, Math.trunc(pageIndex)));
}

export function parsePdfPageInput(
  input: string,
  pageCount: number,
  pageLabels: readonly string[] = [],
): number | undefined {
  const value = input.trim();
  if (value.length === 0 || pageCount <= 0) return undefined;
  const labelIndex = pageLabels.findIndex((label) => label.toLocaleLowerCase() === value.toLocaleLowerCase());
  if (labelIndex >= 0) return clampPdfPageIndex(labelIndex, pageCount);
  const pageNumber = Number(value);
  if (!Number.isInteger(pageNumber) || pageNumber < 1 || pageNumber > pageCount) return undefined;
  return pageNumber - 1;
}

export function pdfWheelZoomFactor(deltaY: number, deltaMode = 0): number {
  if (!Number.isFinite(deltaY) || deltaY === 0) return 1;
  const normalizedDelta = deltaY * (deltaMode === 1 ? 16 : deltaMode === 2 ? 120 : 1);
  const maximumStep = 1.15;
  return Math.max(1 / maximumStep, Math.min(maximumStep, Math.exp(-normalizedDelta * 0.01)));
}

export function pdfWheelZoomFactorForInput(input: PdfWheelZoomInput): number | undefined {
  if (input.nativeGestureActive === true || (input.ctrlKey !== true && input.metaKey !== true)) {
    return undefined;
  }
  return pdfWheelZoomFactor(input.deltaY, input.deltaMode);
}
