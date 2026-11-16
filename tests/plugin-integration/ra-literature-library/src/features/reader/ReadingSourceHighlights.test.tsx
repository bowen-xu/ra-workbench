// @vitest-environment happy-dom
import { act, StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { AttachmentSummary } from "@research-assistant/core/literature";
import type { PdfCharacterSelection } from "../../../../../../apps/web/src/core/components/pdf-view/pdfPageCharacterModel";
import type { ReaderSessionSnapshot } from "../../../../../../../plugins/ra-literature-library/src/features/reader/session/ReaderSessionController";
import type { ReadingFieldSource } from "../../../../../../../plugins/ra-literature-library/src/features/ai/readingFieldSources";
import { readingFieldColor } from "../../../../../../../plugins/ra-literature-library/src/features/ai/readingFieldSources";
import { ReadingSourceHighlights } from "../../../../../../../plugins/ra-literature-library/src/features/reader/ReadingSourceHighlights";

const hash = "a".repeat(64);
const attachment = { id: "pdf", contentSha256: hash } as AttachmentSummary;
function source(id: string, pageIndex = 0): ReadingFieldSource {
  return { id, fieldId: "research_question", fieldTitle: "研究问题", evidence: {
    id, citationKey: id, ordinal: 0, anchorId: "anchor", paperId: "paper", paperTitle: "Paper", attachmentId: "pdf", contentSha256: hash,
    pageIndex, quoteExact: "literal", quotePrefix: "before", quoteSuffix: "after", contextText: "literal", relationKind: "direct_quote",
    validationStatus: "valid", currentAnchorStatus: "exact", createdAt: "now",
  } };
}
function selection(): PdfCharacterSelection {
  const rect = { x: .1, y: .2, width: .3, height: .02 };
  const segment = { pageIndex: 0, start: 0, end: 7, text: "literal", prefix: "before", suffix: "after", rects: [rect], lastClientRect: { left: 1, top: 2, right: 4, bottom: 3, width: 3, height: 1 } };
  return { ...segment, segments: [segment], focusPageIndex: 0, focusRect: rect };
}
let container: HTMLDivElement, page: HTMLDivElement, root: ReturnType<typeof createRoot>;
beforeEach(() => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  container = document.createElement("div"); page = document.createElement("div");
  document.body.append(container, page); root = createRoot(container);
});
afterEach(async () => { await act(async () => root.unmount()); container.remove(); page.remove(); vi.unstubAllGlobals(); });
function fixture() {
  const snapshot: ReaderSessionSnapshot = { ready: true, lifecycle: "active", pageCount: 8, pageIndex: 0, pageLabels: [], scalePercent: 100,
    status: "", outline: [], pageHosts: [{ pageIndex: 0, element: page }, { pageIndex: 6, element: page }], findCurrent: 0, findTotal: 0,
    characterSelectionAvailable: true, resources: { suspended: false, cachedPageIds: [], canvasCount: 0, resourceApiAvailable: true }, canNavigateBack: false, canNavigateForward: false };
  return { subscribe: () => () => undefined, getSnapshot: () => snapshot, resolveQuoteSelection: vi.fn(async (): Promise<PdfCharacterSelection | undefined> => selection()) };
}
it("uses the same field color and bounded worker geometry without creating annotations", async () => {
  const controller = fixture(); const sources = [source("near"), source("far", 6)];
  await act(async () => root.render(<StrictMode><ReadingSourceHighlights sources={sources} attachment={attachment} controller={controller} active /></StrictMode>));
  expect(controller.resolveQuoteSelection).toHaveBeenCalledOnce();
  expect(controller.resolveQuoteSelection).toHaveBeenCalledWith(0, "literal", "before", "after");
  const highlight = page.querySelector<HTMLElement>('.reading-source-highlight');
  expect(highlight?.style.left).toBe("10%");
  expect(highlight?.style.backgroundColor).toBe(readingFieldColor("research_question"));
  expect(page.querySelector('button')).toBeNull();
  await act(async () => root.render(<ReadingSourceHighlights sources={[]} attachment={attachment} controller={controller} active />));
  expect(page.querySelector('.reading-source-highlight')).toBeNull();
});
it("does not paint stale, missing, invalid, empty page-only or replaced attachment evidence", async () => {
  const controller = fixture(); const original = source("source");
  const sources = ["stale", "missing"].map(state => ({ ...original, evidence: { ...original.evidence, currentAnchorStatus: state as ReadingFieldSource["evidence"]["currentAnchorStatus"] } }));
  sources.push({ ...original, evidence: { ...original.evidence, currentAnchorStatus: "page_level", quoteExact: "" } });
  sources.push({ ...original, evidence: { ...original.evidence, validationStatus: "invalid" } });
  sources.push({ ...original, evidence: { ...original.evidence, contentSha256: "b".repeat(64) } });
  await act(async () => root.render(<ReadingSourceHighlights sources={sources} attachment={attachment} controller={controller} active />));
  expect(controller.resolveQuoteSelection).not.toHaveBeenCalled(); expect(page.querySelector('.reading-source-highlight')).toBeNull();
});
it("does not paint ambiguous quotes or a late result after the sources change", async () => {
  const controller = fixture(); const original = source("old");
  controller.resolveQuoteSelection.mockResolvedValueOnce(undefined);
  await act(async () => root.render(<ReadingSourceHighlights sources={[original]} attachment={attachment} controller={controller} active />));
  expect(page.querySelector('.reading-source-highlight')).toBeNull();
  let complete: (selection: PdfCharacterSelection) => void = () => undefined;
  controller.resolveQuoteSelection.mockImplementationOnce(() => new Promise(resolve => { complete = resolve; }));
  await act(async () => root.render(<ReadingSourceHighlights sources={[source("pending")]} attachment={attachment} controller={controller} active />));
  await act(async () => root.render(<ReadingSourceHighlights sources={[]} attachment={attachment} controller={controller} active />));
  await act(async () => complete(selection()));
  expect(page.querySelector('.reading-source-highlight')).toBeNull();
});
it("derives a unique highlight from a page-level excerpt without upgrading its anchor or review status", async () => {
  const controller = fixture(); const original = source("page-only");
  const sources: readonly ReadingFieldSource[] = [{ ...original, evidence: { ...original.evidence, currentAnchorStatus: "page_level" } }];
  await act(async () => root.render(<ReadingSourceHighlights sources={sources} attachment={attachment} controller={controller} active />));
  expect(page.querySelector('.reading-source-highlight')).not.toBeNull();
  expect(sources[0]?.evidence.currentAnchorStatus).toBe("page_level");
});
it("paints the configured template color instead of its default field palette", async () => {
  const controller = fixture();
  await act(async () => root.render(<ReadingSourceHighlights sources={[{ ...source("custom"), color: "#123456" }]} attachment={attachment} controller={controller} active />));
  expect(page.querySelector<HTMLElement>(".reading-source-highlight")?.style.backgroundColor).toBe("#123456");
});
