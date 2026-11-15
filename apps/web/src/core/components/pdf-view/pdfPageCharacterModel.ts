import type { PdfSelectionRect } from "../pdfReaderModel";
import type { PdfSelectionTextLocation } from "@research-assistant/core/pdf-view";

export interface PdfNormalizedRect {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

export interface PdfSourceCharacter {
  readonly value: string;
  readonly rect: PdfNormalizedRect;
  readonly rotation: number;
  readonly lineBreakAfter?: boolean;
}

export interface PdfPageCharacter {
  readonly value: string;
  readonly pageIndex: number;
  readonly rect: PdfNormalizedRect;
  readonly inlineRect: PdfNormalizedRect;
  readonly rotation: number;
  readonly vertical: boolean;
  readonly lineIndex: number;
  readonly blockIndex: number;
  readonly lineBreakAfter: boolean;
}

export interface PdfPageLine {
  readonly index: number;
  readonly characterIndexes: readonly number[];
  readonly rect: PdfNormalizedRect;
}

export interface PdfPageCharacterModel {
  readonly pageIndex: number;
  readonly version: number;
  readonly text: string;
  readonly chars: readonly PdfPageCharacter[];
  readonly lines: readonly PdfPageLine[];
  readonly pageElement: HTMLElement;
}

export interface PdfCharacterOffset {
  readonly index: number;
  readonly offset: 0 | 1;
}

export interface PdfCharacterSelection {
  readonly pageIndex: number;
  readonly start: number;
  readonly end: number;
  readonly text: string;
  readonly prefix: string;
  readonly suffix: string;
  readonly rects: readonly PdfNormalizedRect[];
  readonly lastClientRect: PdfSelectionRect;
  readonly segments: readonly PdfCharacterSelectionSegment[];
  readonly focusPageIndex: number;
  readonly focusRect: PdfNormalizedRect;
}

export interface PdfCharacterSelectionSegment {
  readonly location?: PdfSelectionTextLocation;
  readonly context?: { readonly before: string; readonly after: string };
  readonly pageIndex: number;
  readonly start: number;
  readonly end: number;
  readonly text: string;
  readonly prefix: string;
  readonly suffix: string;
  readonly rects: readonly PdfNormalizedRect[];
  readonly lastClientRect: PdfSelectionRect;
}

export interface PdfTextLayerOffsetMapper {
  characterIndexForDomOffset(pageIndex: number, node: Node, offset: number): number | undefined;
}

export type PdfSelectionGranularity = "character" | "word" | "line" | "all";

interface PdfPageSourceEntry {
  readonly chars: readonly PdfSourceCharacter[];
  readonly revision: number;
}

interface PdfPageCacheEntry {
  readonly model: PdfPageCharacterModel;
  readonly sourceRevision: number;
  readonly width: number;
  readonly height: number;
}

interface MutableCharacter extends Omit<PdfPageCharacter, "inlineRect" | "lineIndex"> {
  inlineRect: PdfNormalizedRect;
  lineIndex: number;
}

const GEOMETRY_EPSILON = 0.000_001;

function normalizedRotation(rotation: number): number {
  return (Math.round(rotation) % 360 + 360) % 360;
}

function isVerticalRotation(rotation: number): boolean {
  const value = normalizedRotation(rotation);
  return value === 90 || value === 270;
}

export function visiblePdfCharacterRect(
  character: Pick<PdfPageCharacter, "rect" | "inlineRect" | "vertical">,
): PdfNormalizedRect {
  if (character.vertical) {
    return {
      x: character.inlineRect.x,
      y: character.rect.y,
      width: character.inlineRect.width,
      height: character.rect.height,
    };
  }
  return {
    x: character.rect.x,
    y: character.inlineRect.y,
    width: character.rect.width,
    height: character.inlineRect.height,
  };
}

function fallbackWhitespaceRect(
  previous: PdfNormalizedRect | undefined,
  next: PdfNormalizedRect | undefined,
): PdfNormalizedRect | undefined {
  if (previous === undefined || next === undefined) return previous ?? next;
  const sameLine = Math.min(previous.y + previous.height, next.y + next.height) - Math.max(previous.y, next.y)
    >= Math.min(previous.height, next.height) * 0.55;
  if (!sameLine) return previous;
  return {
    x: previous.x + previous.width,
    y: Math.min(previous.y, next.y),
    width: Math.max(GEOMETRY_EPSILON, next.x - previous.x - previous.width),
    height: Math.max(previous.height, next.height),
  };
}

function separatorBetween(
  previous: MutableCharacter | undefined,
  next: MutableCharacter | undefined,
): "" | " " | "\n" {
  if (previous === undefined || next === undefined || /\s/u.test(previous.value) || /\s/u.test(next.value)) return "";
  if (previous.lineBreakAfter) return "\n";
  const previousRect = visiblePdfCharacterRect(previous);
  const nextRect = visiblePdfCharacterRect(next);
  if (previous.vertical !== next.vertical) return "\n";
  const vertical = previous.vertical;
  const crossStart = vertical ? "x" : "y", crossSize = vertical ? "width" : "height";
  const inlineStart = vertical ? "y" : "x", inlineSize = vertical ? "height" : "width";
  const overlap = Math.min(previousRect[crossStart] + previousRect[crossSize], nextRect[crossStart] + nextRect[crossSize])
    - Math.max(previousRect[crossStart], nextRect[crossStart]);
  if (overlap < Math.min(previousRect[crossSize], nextRect[crossSize]) * 0.55) return "\n";
  const backwards = previous.rotation === 180 || previous.rotation === 270;
  const gap = backwards ? previousRect[inlineStart] - nextRect[inlineStart] - nextRect[inlineSize]
    : nextRect[inlineStart] - previousRect[inlineStart] - previousRect[inlineSize];
  return gap > Math.max(previousRect[crossSize], nextRect[crossSize]) * 0.15 ? " " : "";
}

function assignLines(chars: MutableCharacter[]): readonly PdfPageLine[] {
  const lines: Array<{ indexes: number[]; rect: PdfNormalizedRect; vertical: boolean }> = [];
  chars.forEach((char, index) => {
    if (/\s/u.test(char.value)) return;
    const rect = char.rect;
    const center = char.vertical ? rect.x + rect.width / 2 : rect.y + rect.height / 2;
    let bestLine = -1;
    let bestDistance = Number.POSITIVE_INFINITY;
    lines.forEach((line, lineIndex) => {
      if (line.vertical !== char.vertical) return;
      const lineCenter = char.vertical
        ? line.rect.x + line.rect.width / 2
        : line.rect.y + line.rect.height / 2;
      const overlap = char.vertical
        ? Math.min(line.rect.x + line.rect.width, rect.x + rect.width) - Math.max(line.rect.x, rect.x)
        : Math.min(line.rect.y + line.rect.height, rect.y + rect.height) - Math.max(line.rect.y, rect.y);
      const minimumSize = char.vertical
        ? Math.min(line.rect.width, rect.width)
        : Math.min(line.rect.height, rect.height);
      const blockSize = char.vertical
        ? Math.max(line.rect.width, rect.width)
        : Math.max(line.rect.height, rect.height);
      const inlineGap = char.vertical
        ? Math.max(0, line.rect.y - (rect.y + rect.height), rect.y - (line.rect.y + line.rect.height))
        : Math.max(0, line.rect.x - (rect.x + rect.width), rect.x - (line.rect.x + line.rect.width));
      const distance = Math.abs(center - lineCenter);
      // Keep distant columns in independent line fragments. If they share one
      // inlineRect, a slightly different baseline in the other column inflates
      // the painted selection band around the current column.
      if (overlap >= minimumSize * 0.55 && inlineGap <= blockSize * 4 && distance < bestDistance) {
        bestLine = lineIndex;
        bestDistance = distance;
      }
    });
    if (bestLine < 0) {
      bestLine = lines.length;
      lines.push({ indexes: [], rect, vertical: char.vertical });
    }
    const line = lines[bestLine];
    if (line === undefined) return;
    line.indexes.push(index);
    const left = Math.min(line.rect.x, rect.x);
    const top = Math.min(line.rect.y, rect.y);
    const right = Math.max(line.rect.x + line.rect.width, rect.x + rect.width);
    const bottom = Math.max(line.rect.y + line.rect.height, rect.y + rect.height);
    line.rect = { x: left, y: top, width: right - left, height: bottom - top };
    char.lineIndex = bestLine;
  });

  let adjacentLine = -1;
  chars.forEach((char, index) => {
    if (char.lineIndex >= 0) adjacentLine = char.lineIndex;
    else if (adjacentLine >= 0) {
      char.lineIndex = adjacentLine;
      lines[adjacentLine]?.indexes.push(index);
    }
  });
  adjacentLine = -1;
  for (let index = chars.length - 1; index >= 0; index -= 1) {
    const char = chars[index];
    if (char === undefined) continue;
    if (char.lineIndex >= 0) adjacentLine = char.lineIndex;
    else if (adjacentLine >= 0) {
      char.lineIndex = adjacentLine;
      lines[adjacentLine]?.indexes.push(index);
    }
  }

  lines.forEach((line, lineIndex) => {
    line.indexes.sort((left, right) => left - right);
    line.indexes.forEach((characterIndex) => {
      const char = chars[characterIndex];
      if (char === undefined) return;
      char.inlineRect = char.vertical
        ? { ...char.rect, x: line.rect.x, width: line.rect.width }
        : { ...char.rect, y: line.rect.y, height: line.rect.height };
      char.lineIndex = lineIndex;
    });
  });
  return lines.map((line, index) => ({ index, characterIndexes: line.indexes, rect: line.rect }));
}

function buildPageModel(
  pageElement: HTMLElement,
  pageIndex: number,
  version: number,
  sourceChars: readonly PdfSourceCharacter[],
): PdfPageCharacterModel {
  const chars: MutableCharacter[] = [];
  sourceChars.forEach((source, blockIndex) => {
    if (source.rect.width <= 0 || source.rect.height <= 0 || source.value.length === 0) return;
    const rotation = normalizedRotation(source.rotation);
    const next: MutableCharacter = {
      value: source.value,
      pageIndex,
      rect: source.rect,
      inlineRect: source.rect,
      rotation,
      vertical: isVerticalRotation(rotation),
      lineIndex: -1,
      blockIndex,
      lineBreakAfter: source.lineBreakAfter === true,
    };
    const separator = separatorBetween(chars.at(-1), next);
    if (separator.length > 0) {
      const previous = chars.at(-1);
      if (previous !== undefined) {
        const separatorRect = fallbackWhitespaceRect(previous.rect, next.rect) ?? previous.rect;
        chars.push({
          ...previous,
          value: separator,
          rect: separatorRect,
          inlineRect: separatorRect,
          lineIndex: -1,
          blockIndex,
          lineBreakAfter: false,
        });
      }
    }
    chars.push(next);
  });
  const lines = assignLines(chars);
  return {
    pageIndex,
    version,
    text: chars.map((char) => char.value).join(""),
    chars,
    lines,
    pageElement,
  };
}

export class PdfPageCharacterStore {
  private readonly sources = new Map<number, PdfPageSourceEntry>();
  private readonly pages = new Map<number, PdfPageCacheEntry>();
  private nextVersion = 1;
  private nextSourceRevision = 1;

  get _pdfPages(): ReadonlyMap<number, PdfPageCharacterModel> {
    return new Map([...this.pages].map(([pageIndex, entry]) => [pageIndex, entry.model]));
  }

  setPageCharacters(pageIndex: number, chars: readonly PdfSourceCharacter[]): void {
    this.sources.set(pageIndex, { chars, revision: this.nextSourceRevision });
    this.nextSourceRevision += 1;
    this.pages.delete(pageIndex);
  }

  hasPageCharacters(pageIndex: number): boolean {
    return this.sources.has(pageIndex);
  }

  syncPage(pageElement: HTMLElement): PdfPageCharacterModel | undefined {
    const pageNumber = Number(pageElement.dataset.pageNumber);
    if (!Number.isInteger(pageNumber) || pageNumber < 1) return undefined;
    const pageIndex = pageNumber - 1;
    const source = this.sources.get(pageIndex);
    const pageRect = pdfPageContentRect(pageElement);
    if (source === undefined || source.chars.length === 0 || pageRect.width <= 0 || pageRect.height <= 0) return undefined;
    const current = this.pages.get(pageIndex);
    if (current !== undefined
      && current.model.pageElement === pageElement
      && current.sourceRevision === source.revision
      && Math.abs(current.width - pageRect.width) < 0.5
      && Math.abs(current.height - pageRect.height) < 0.5) {
      return current.model;
    }
    const model = buildPageModel(pageElement, pageIndex, this.nextVersion, source.chars);
    this.nextVersion += 1;
    this.pages.set(pageIndex, {
      model,
      sourceRevision: source.revision,
      width: pageRect.width,
      height: pageRect.height,
    });
    pageElement.dataset.pdfCharacterModelVersion = String(model.version);
    return model;
  }

  get(pageIndex: number): PdfPageCharacterModel | undefined {
    return this.pages.get(pageIndex)?.model;
  }

  invalidate(pageIndex: number): void {
    this.pages.delete(pageIndex);
  }

  releasePage(pageIndex: number): void {
    this.sources.delete(pageIndex);
    this.pages.delete(pageIndex);
  }

  clear(): void {
    this.sources.clear();
    this.pages.clear();
  }
}

export function pdfPageContentRect(pageElement: HTMLElement): DOMRect {
  const contentElement = pageElement.querySelector<HTMLElement>(":scope > .canvasWrapper")
    ?? pageElement.querySelector<HTMLElement>(":scope > .pdf-view-overlay-layer")
    ?? pageElement;
  return contentElement.getBoundingClientRect();
}
