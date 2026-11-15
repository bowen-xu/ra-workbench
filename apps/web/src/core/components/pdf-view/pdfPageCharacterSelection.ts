import type { PdfSelectionRect } from "../pdfReaderModel";
import {
  pdfPageContentRect,
  visiblePdfCharacterRect,
  type PdfCharacterOffset,
  type PdfCharacterSelection,
  type PdfCharacterSelectionSegment,
  type PdfNormalizedRect,
  type PdfPageCharacter,
  type PdfPageCharacterModel,
  type PdfSelectionGranularity,
} from "./pdfPageCharacterModel";

interface PdfCharacterRange {
  readonly start: number;
  readonly end: number;
}

function pointDistanceSquared(rect: PdfNormalizedRect, x: number, y: number): number {
  const dx = x < rect.x ? rect.x - x : x > rect.x + rect.width ? x - rect.x - rect.width : 0;
  const dy = y < rect.y ? rect.y - y : y > rect.y + rect.height ? y - rect.y - rect.height : 0;
  return dx * dx + dy * dy;
}

function normalizedRotation(rotation: number): 0 | 90 | 180 | 270 {
  const normalized = (Math.round(rotation / 90) * 90 % 360 + 360) % 360;
  if (normalized === 90 || normalized === 180 || normalized === 270) return normalized;
  return 0;
}

function inlineCoordinate(character: PdfPageCharacter, x: number, y: number): number {
  return character.vertical ? y : x;
}

function characterCenter(character: PdfPageCharacter): number {
  const rect = visiblePdfCharacterRect(character);
  return character.vertical ? rect.y + rect.height / 2 : rect.x + rect.width / 2;
}

function defaultProgressionIsPositive(character: PdfPageCharacter): boolean {
  const rotation = normalizedRotation(character.rotation);
  return rotation === 0 || rotation === 90;
}

function progressionIsPositive(model: PdfPageCharacterModel, index: number): boolean {
  const character = model.chars[index];
  if (character === undefined) return true;
  const adjacentIndexes = [index + 1, index - 1];
  for (const adjacentIndex of adjacentIndexes) {
    const adjacent = model.chars[adjacentIndex];
    if (adjacent === undefined || /\s/u.test(adjacent.value) || adjacent.lineIndex !== character.lineIndex) continue;
    const delta = adjacentIndex > index
      ? characterCenter(adjacent) - characterCenter(character)
      : characterCenter(character) - characterCenter(adjacent);
    if (Math.abs(delta) > 0.000_001) return delta > 0;
  }
  return defaultProgressionIsPositive(character);
}

function clampedCharacterOffset(model: PdfPageCharacterModel, point: PdfCharacterOffset): PdfCharacterOffset {
  if (model.chars.length === 0) return { index: 0, offset: 0 };
  return {
    index: Math.max(0, Math.min(model.chars.length - 1, Math.trunc(point.index))),
    offset: point.offset === 1 ? 1 : 0,
  };
}

function boundaryIndex(model: PdfPageCharacterModel, point: PdfCharacterOffset): number {
  const clamped = clampedCharacterOffset(model, point);
  return Math.max(0, Math.min(model.chars.length, clamped.index + clamped.offset));
}

export function beforePdfCharacter(index: number): PdfCharacterOffset {
  return { index, offset: 0 };
}

export function afterPdfCharacter(index: number): PdfCharacterOffset {
  return { index, offset: 1 };
}

export function hitTestPdfCharacterOffset(
  model: PdfPageCharacterModel,
  clientX: number,
  clientY: number,
): PdfCharacterOffset | undefined {
  const contentRect = pdfPageContentRect(model.pageElement);
  if (contentRect.width <= 0 || contentRect.height <= 0) return undefined;
  const x = (clientX - contentRect.left) / contentRect.width;
  const y = (clientY - contentRect.top) / contentRect.height;
  let closestIndex: number | undefined;
  let closestDistance = Number.POSITIVE_INFINITY;
  model.chars.forEach((character, index) => {
    if (/\s/u.test(character.value) || character.rect.width <= 0 || character.rect.height <= 0) return;
    const distance = pointDistanceSquared(visiblePdfCharacterRect(character), x, y);
    if (distance < closestDistance) {
      closestIndex = index;
      closestDistance = distance;
    }
  });
  if (closestIndex === undefined) return undefined;
  const character = model.chars[closestIndex];
  if (character === undefined) return undefined;
  const coordinate = inlineCoordinate(character, x, y);
  const isAfter = progressionIsPositive(model, closestIndex)
    ? coordinate >= characterCenter(character)
    : coordinate < characterCenter(character);
  return { index: closestIndex, offset: isAfter ? 1 : 0 };
}

export function normalizePdfCharacterRange(
  model: PdfPageCharacterModel,
  anchor: PdfCharacterOffset,
  focus: PdfCharacterOffset,
): PdfCharacterRange | undefined {
  let start = Math.min(boundaryIndex(model, anchor), boundaryIndex(model, focus));
  let end = Math.max(boundaryIndex(model, anchor), boundaryIndex(model, focus));
  while (start < end && /\s/u.test(model.chars[start]?.value ?? "")) start += 1;
  while (end > start && /\s/u.test(model.chars[end - 1]?.value ?? "")) end -= 1;
  return start < end ? { start, end } : undefined;
}

function characterTextOffsets(model: PdfPageCharacterModel): readonly PdfCharacterRange[] {
  let offset = 0;
  return model.chars.map((character) => {
    const start = offset;
    offset += character.value.length;
    return { start, end: offset };
  });
}

function indexesForTextRange(
  offsets: readonly PdfCharacterRange[],
  start: number,
  end: number,
): PdfCharacterRange | undefined {
  const first = offsets.findIndex((offset) => offset.end > start);
  let last = -1;
  offsets.forEach((offset, index) => { if (offset.start < end) last = index; });
  return first >= 0 && last >= first ? { start: first, end: last + 1 } : undefined;
}

export function expandPdfCharacterRange(
  model: PdfPageCharacterModel,
  characterIndex: number,
  granularity: PdfSelectionGranularity,
): PdfCharacterRange | undefined {
  if (model.chars.length === 0) return undefined;
  const index = Math.max(0, Math.min(model.chars.length - 1, Math.trunc(characterIndex)));
  if (granularity === "character") return { start: index, end: index + 1 };
  if (granularity === "all") return { start: 0, end: model.chars.length };
  if (granularity === "line") {
    const lineIndex = model.chars[index]?.lineIndex;
    const indexes = lineIndex === undefined ? undefined : model.lines[lineIndex]?.characterIndexes;
    if (indexes === undefined || indexes.length === 0) return undefined;
    return { start: indexes[0] ?? index, end: (indexes.at(-1) ?? index) + 1 };
  }

  const offsets = characterTextOffsets(model);
  const targetOffset = offsets[index]?.start ?? 0;
  if (typeof Intl.Segmenter === "function") {
    const segmenter = new Intl.Segmenter(undefined, { granularity: "word" });
    for (const segment of segmenter.segment(model.text)) {
      if (targetOffset >= segment.index && targetOffset < segment.index + segment.segment.length) {
        if (segment.isWordLike || /[\p{Letter}\p{Number}\p{Mark}\u00ad]/u.test(segment.segment)) {
          return indexesForTextRange(offsets, segment.index, segment.index + segment.segment.length);
        }
        break;
      }
    }
  }
  const isWordCharacter = (value: string): boolean => /[\p{Letter}\p{Number}\p{Mark}\u00ad]/u.test(value);
  if (!isWordCharacter(model.chars[index]?.value ?? "")) return { start: index, end: index + 1 };
  let start = index;
  let end = index + 1;
  while (start > 0 && isWordCharacter(model.chars[start - 1]?.value ?? "")) start -= 1;
  while (end < model.chars.length && isWordCharacter(model.chars[end]?.value ?? "")) end += 1;
  return { start, end };
}

function boundingRect(rectangles: readonly PdfNormalizedRect[]): PdfNormalizedRect | undefined {
  const first = rectangles[0];
  if (first === undefined) return undefined;
  let left = first.x;
  let top = first.y;
  let right = first.x + first.width;
  let bottom = first.y + first.height;
  for (const rect of rectangles.slice(1)) {
    left = Math.min(left, rect.x);
    top = Math.min(top, rect.y);
    right = Math.max(right, rect.x + rect.width);
    bottom = Math.max(bottom, rect.y + rect.height);
  }
  return { x: left, y: top, width: right - left, height: bottom - top };
}

function structuredSelectionRects(chars: readonly PdfPageCharacter[]): readonly PdfNormalizedRect[] {
  const lines: PdfPageCharacter[][] = [];
  let current: PdfPageCharacter[] = [];
  const flush = (): void => {
    if (current.length > 0) lines.push(current);
    current = [];
  };
  for (const character of chars) {
    if (/\s/u.test(character.value)) {
      if (character.value.includes("\n")) flush();
      continue;
    }
    const previous = current.at(-1);
    if (previous !== undefined && (
      previous.lineBreakAfter
      || previous.lineIndex !== character.lineIndex
      || previous.vertical !== character.vertical
    )) {
      flush();
    }
    current.push(character);
    if (character.lineBreakAfter) flush();
  }
  flush();
  return lines.flatMap((line) => {
    const rect = boundingRect(line.map((character) => character.inlineRect));
    return rect === undefined ? [] : [rect];
  });
}

function clientRect(pageRect: DOMRect, rect: PdfNormalizedRect): PdfSelectionRect {
  return {
    left: pageRect.left + rect.x * pageRect.width,
    top: pageRect.top + rect.y * pageRect.height,
    right: pageRect.left + (rect.x + rect.width) * pageRect.width,
    bottom: pageRect.top + (rect.y + rect.height) * pageRect.height,
    width: rect.width * pageRect.width,
    height: rect.height * pageRect.height,
  };
}

function createSegment(model: PdfPageCharacterModel, range: PdfCharacterRange): PdfCharacterSelectionSegment | undefined {
  const selectedChars = model.chars.slice(range.start, range.end);
  const rects = structuredSelectionRects(selectedChars);
  const lastRect = rects.at(-1);
  if (lastRect === undefined) return undefined;
  const pageRect = pdfPageContentRect(model.pageElement);
  const start = model.chars[range.start], end = model.chars[range.end - 1];
  const startColumn = start ? model.lines[start.lineIndex]?.characterIndexes.indexOf(range.start) : undefined;
  const endColumn = end ? model.lines[end.lineIndex]?.characterIndexes.indexOf(range.end - 1) : undefined;
  const location = start && end && startColumn !== undefined && startColumn >= 0 && endColumn !== undefined && endColumn >= 0
    ? { startLine: start.lineIndex + 1, startColumn: startColumn + 1, endLine: end.lineIndex + 1, endColumn: endColumn + 1 } : undefined;
  return {
    ...(location ? { location } : {}),
    pageIndex: model.pageIndex,
    start: range.start,
    end: range.end,
    text: selectedChars.map((character) => character.value).join("").replace(/\u00ad\n?/gu, "").trim(),
    prefix: model.chars.slice(0, range.start).map((character) => character.value).join("").slice(-64),
    suffix: model.chars.slice(range.end).map((character) => character.value).join("").slice(0, 64),
    context: {
      before: model.chars.slice(Math.max(0, range.start - 600), range.start).map(character => character.value).join("").slice(-600),
      after: model.chars.slice(range.end, range.end + 600).map(character => character.value).join("").slice(0, 600),
    },
    rects,
    lastClientRect: clientRect(pageRect, lastRect),
  };
}

export function createPdfCharacterSelection(
  model: PdfPageCharacterModel,
  anchor: PdfCharacterOffset,
  focus: PdfCharacterOffset,
): PdfCharacterSelection | undefined {
  const range = normalizePdfCharacterRange(model, anchor, focus);
  if (range === undefined) return undefined;
  const segment = createSegment(model, range);
  if (segment === undefined) return undefined;
  const focusPoint = clampedCharacterOffset(model, focus);
  const focusCharacter = model.chars[focusPoint.index];
  const focusRect = focusCharacter === undefined ? segment.rects.at(-1) : visiblePdfCharacterRect(focusCharacter);
  if (focusRect === undefined) return undefined;
  return {
    ...segment,
    lastClientRect: clientRect(pdfPageContentRect(model.pageElement), focusRect),
    segments: [segment],
    focusPageIndex: model.pageIndex,
    focusRect,
  };
}

function characterSelectionRanges(
  anchorModel: PdfPageCharacterModel,
  anchor: PdfCharacterOffset,
  focusModel: PdfPageCharacterModel,
  focus: PdfCharacterOffset,
): readonly { readonly model: PdfPageCharacterModel; readonly range: PdfCharacterRange }[] {
  if (anchorModel.pageIndex === focusModel.pageIndex) {
    const range = normalizePdfCharacterRange(anchorModel, anchor, focus);
    return range ? [{ model: anchorModel, range }] : [];
  }
  if (Math.abs(anchorModel.pageIndex - focusModel.pageIndex) !== 1) return [];
  const forward = anchorModel.pageIndex < focusModel.pageIndex;
  const firstModel = forward ? anchorModel : focusModel;
  const secondModel = forward ? focusModel : anchorModel;
  const firstPoint = forward ? anchor : focus;
  const secondPoint = forward ? focus : anchor;
  const firstRange = normalizePdfCharacterRange(
    firstModel,
    firstPoint,
    afterPdfCharacter(firstModel.chars.length - 1),
  );
  const secondRange = normalizePdfCharacterRange(secondModel, beforePdfCharacter(0), secondPoint);
  return [...(firstRange ? [{ model: firstModel, range: firstRange }] : []),
    ...(secondRange ? [{ model: secondModel, range: secondRange }] : [])];
}

/** Live feedback uses the same ranges and rectangles as commit, without
 * materializing quote text, surrounding context or client-space popup anchors. */
export function createPdfLiveCharacterSegments(
  anchorModel: PdfPageCharacterModel, anchor: PdfCharacterOffset,
  focusModel: PdfPageCharacterModel, focus: PdfCharacterOffset,
): readonly { readonly pageIndex: number; readonly rects: readonly PdfNormalizedRect[] }[] {
  return characterSelectionRanges(anchorModel, anchor, focusModel, focus).flatMap(({ model, range }) => {
    const rects = structuredSelectionRects(model.chars.slice(range.start, range.end));
    return rects.length ? [{ pageIndex: model.pageIndex, rects }] : [];
  });
}

export function createPdfMultiPageCharacterSelection(
  anchorModel: PdfPageCharacterModel,
  anchor: PdfCharacterOffset,
  focusModel: PdfPageCharacterModel,
  focus: PdfCharacterOffset,
): PdfCharacterSelection | undefined {
  if (anchorModel.pageIndex === focusModel.pageIndex) return createPdfCharacterSelection(anchorModel, anchor, focus);
  const segments = characterSelectionRanges(anchorModel, anchor, focusModel, focus)
    .map(({ model, range }) => createSegment(model, range))
    .filter((segment): segment is PdfCharacterSelectionSegment => segment !== undefined);
  if (segments.length === 0) return undefined;
  const focusPoint = clampedCharacterOffset(focusModel, focus);
  const focusCharacter = focusModel.chars[focusPoint.index];
  const focusRect = focusCharacter === undefined ? segments.at(-1)?.rects.at(-1) : visiblePdfCharacterRect(focusCharacter);
  const firstSegment = segments[0];
  const lastSegment = segments.at(-1);
  if (focusRect === undefined || firstSegment === undefined || lastSegment === undefined) return undefined;
  return {
    pageIndex: firstSegment.pageIndex,
    start: firstSegment.start,
    end: lastSegment.end,
    text: segments.map((segment) => segment.text).join("\n"),
    prefix: firstSegment.prefix,
    suffix: lastSegment.suffix,
    rects: firstSegment.rects,
    lastClientRect: clientRect(pdfPageContentRect(focusModel.pageElement), focusRect),
    segments,
    focusPageIndex: focusModel.pageIndex,
    focusRect,
  };
}
