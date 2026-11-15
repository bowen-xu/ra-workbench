import type { PDFPageProxy } from "pdfjs-dist/legacy/build/pdf.mjs";

import type { PdfSourceCharacter } from "./pdfPageCharacterModel";

export type PageTextContent = Awaited<ReturnType<PDFPageProxy["getTextContent"]>>;
type PdfPageViewport = ReturnType<PDFPageProxy["getViewport"]>;

interface PdfWorkerCharacter {
  readonly c: string;
  readonly rect: readonly [number, number, number, number];
  readonly rotation: number;
}

function isPdfWorkerCharacter(value: unknown): value is PdfWorkerCharacter {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<PdfWorkerCharacter>;
  return typeof candidate.c === "string"
    && Array.isArray(candidate.rect)
    && candidate.rect.length === 4
    && candidate.rect.every((coordinate) => typeof coordinate === "number" && Number.isFinite(coordinate))
    && typeof candidate.rotation === "number"
    && Number.isFinite(candidate.rotation);
}

function workerCharacters(item: PageTextContent["items"][number]): readonly PdfWorkerCharacter[] {
  const candidate = item as typeof item & { readonly chars?: unknown };
  return Array.isArray(candidate.chars) ? candidate.chars.filter(isPdfWorkerCharacter) : [];
}

export function pdfWorkerCharacterCapability(
  content: PageTextContent,
): "available" | "missing" | "not-required" {
  // PDF.js may emit standalone whitespace items without glyph geometry. Those
  // items are layout separators, not selectable characters, and the page model
  // reconstructs them from adjacent glyph positions.
  const textItems = content.items.filter((item) => "str" in item && /\S/u.test(item.str));
  if (textItems.length === 0) return "not-required";
  return textItems.every((item) => workerCharacters(item).length > 0) ? "available" : "missing";
}

export async function readPdfTextContent(page: PDFPageProxy, options?: Parameters<PDFPageProxy["streamTextContent"]>[0]): Promise<PageTextContent> {
  const reader = page.streamTextContent(options).getReader();
  const result: PageTextContent = {
    items: [],
    styles: Object.create(null) as PageTextContent["styles"],
    lang: null,
  };

  while (true) {
    const chunk = await reader.read();
    if (chunk.done) return result;
    const value = chunk.value as PageTextContent;
    result.lang ??= value.lang;
    Object.assign(result.styles, value.styles);
    result.items.push(...value.items);
  }
}

export function pdfSourceCharacters(
  content: PageTextContent,
  viewport: PdfPageViewport,
): readonly PdfSourceCharacter[] {
  const output: PdfSourceCharacter[] = [];
  for (const item of content.items) {
    if (!("str" in item)) continue;
    const chars = workerCharacters(item);
    chars.forEach((char, index) => {
      const first = viewport.convertToViewportPoint(char.rect[0], char.rect[1]);
      const second = viewport.convertToViewportPoint(char.rect[2], char.rect[3]);
      const left = Math.min(first[0] ?? 0, second[0] ?? 0);
      const top = Math.min(first[1] ?? 0, second[1] ?? 0);
      const right = Math.max(first[0] ?? 0, second[0] ?? 0);
      const bottom = Math.max(first[1] ?? 0, second[1] ?? 0);
      if (viewport.width <= 0 || viewport.height <= 0 || right <= left || bottom <= top) return;
      output.push({
        value: char.c,
        rect: {
          x: left / viewport.width,
          y: top / viewport.height,
          width: (right - left) / viewport.width,
          height: (bottom - top) / viewport.height,
        },
        rotation: char.rotation + viewport.rotation,
        lineBreakAfter: item.hasEOL && index === chars.length - 1,
      });
    });
  }
  return output;
}
