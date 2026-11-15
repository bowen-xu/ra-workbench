import { AnnotationType } from "pdfjs-dist/legacy/build/pdf.mjs";
import type { EventBus, PDFPageView, PDFViewer } from "pdfjs-dist/legacy/web/pdf_viewer.mjs";
import { afterPdfCharacter, beforePdfCharacter, createPdfCharacterSelection } from "./pdfPageCharacterSelection";
import type { PdfPageCharacterModel, PdfPageCharacterStore } from "./pdfPageCharacterModel";

/** Detection supplies text offsets only; geometry always comes from the selection model. */
export function pdfAutoLinkRanges(model: PdfPageCharacterModel) {
  const candidates = /(?:https?:\/\/|www\.)[^\s<>]+|(?:mailto:)?[\p{L}\p{N}.!#$%&'*+/=?^_`{|}~-]+@[\p{L}\p{N}](?:[\p{L}\p{N}.-]*[\p{L}\p{N}])?\.[\p{L}]{2,}/giu;
  const offsets: number[] = [];
  let offset = 0;
  for (const char of model.chars) { offsets.push(offset); offset += char.value.length; }
  return [...model.text.matchAll(candidates)].flatMap((match) => {
    // Exclude prose punctuation, but keep balanced URL parentheses.
    let text = match[0].replace(/[.,;:!?]+$/u, "");
    while (text.endsWith(")") && (text.match(/\)/g)?.length ?? 0) > (text.match(/\(/g)?.length ?? 0)) text = text.slice(0, -1);
    const start = match.index;
    const end = start + text.length;
    const first = offsets.findIndex((value, index) => value + model.chars[index]!.value.length > start);
    const next = offsets.findIndex((value) => value >= end);
    const last = next < 0 ? offsets.length - 1 : next - 1;
    if (first < 0 || last < first) return [];
    let url: URL;
    try { url = new URL(/^www\./i.test(text) ? `https://${text}` : /^(https?:|mailto:)/i.test(text) ? text : `mailto:${text}`); }
    catch { return []; }
    if (!["https:", "http:", "mailto:"].includes(url.protocol)) return [];
    const selection = createPdfCharacterSelection(model, beforePdfCharacter(first), afterPdfCharacter(last));
    return selection ? [{ url: url.href, rects: selection.rects }] : [];
  });
}

async function renderAutoLinks(page: PDFPageView, store: PdfPageCharacterStore, load: () => Promise<void>, isCurrent: () => boolean): Promise<void> {
  const builder = page.annotationLayer;
  if (!builder) return;
  await load();
  if (!isCurrent() || page.annotationLayer !== builder || builder._cancelled) return;
  const model = store.syncPage(page.div);
  if (!model) return; // No glyph data: keep native annotations, never guess text geometry.
  const viewport = page.viewport;
  const annotations = pdfAutoLinkRanges(model).flatMap((link, index) => link.rects.map((rect, part) => {
    const a = viewport.convertToPdfPoint(rect.x * viewport.width, rect.y * viewport.height);
    const b = viewport.convertToPdfPoint((rect.x + rect.width) * viewport.width, (rect.y + rect.height) * viewport.height);
    return {
      id: `glyph_link_${page.id}_${index}_${part}`, annotationType: AnnotationType.LINK,
      url: link.url, unsafeUrl: link.url, rotation: 0, borderStyle: null,
      rect: [Math.min(a[0]!, b[0]!), Math.min(a[1]!, b[1]!), Math.max(a[0]!, b[0]!), Math.max(a[1]!, b[1]!)],
    };
  }));
  // PDF.js owns rendering, URL handling and exclusion of overlapping native annotations.
  await builder.injectLinkAnnotations(annotations);
}

export function bindPdfAutoLinks(eventBus: EventBus, viewer: PDFViewer, store: PdfPageCharacterStore,
  load: (pageIndex: number) => Promise<void>, isCurrent: () => boolean, onError: (reason: unknown) => void): () => void {
  const rendered = ({ pageNumber, error }: { pageNumber: number; error?: unknown }): void => {
    if (error || !isCurrent()) return;
    const page = viewer.getPageView(pageNumber - 1);
    if (page) void renderAutoLinks(page, store, () => load(pageNumber - 1), isCurrent).catch(onError);
  };
  eventBus.on("annotationlayerrendered", rendered);
  return () => eventBus.off("annotationlayerrendered", rendered);
}
