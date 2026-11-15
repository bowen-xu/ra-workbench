export const PDF_RECENT_PAGE_BUFFER = 2;
export const PDF_BACKGROUND_SUSPEND_MS = 60_000;

export interface PdfCachedPageView {
  readonly id: number;
  destroy(): void;
}

export interface PdfVisiblePageSet {
  readonly ids: ReadonlySet<number>;
}

export interface PdfViewerResourceApi {
  getCachedPageViews(): ReadonlySet<PdfCachedPageView>;
  _getVisiblePages(): PdfVisiblePageSet;
}

export interface PdfPageBufferResult {
  readonly destroyedPageIds: readonly number[];
  readonly retainedPageIds: readonly number[];
}

export function supportsPdfViewerResourceApi(value: unknown): value is PdfViewerResourceApi {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<PdfViewerResourceApi>;
  return typeof candidate.getCachedPageViews === "function"
    && typeof candidate._getVisiblePages === "function";
}

export function trimPdfPageBuffer(
  viewer: PdfViewerResourceApi,
  recentPageCount = PDF_RECENT_PAGE_BUFFER,
  onDestroy?: (pageId: number) => void,
): PdfPageBufferResult {
  const visibleIds = viewer._getVisiblePages().ids;
  const cached = [...viewer.getCachedPageViews()];
  if (visibleIds.size === 0) {
    return { destroyedPageIds: [], retainedPageIds: cached.map((pageView) => pageView.id) };
  }
  const protectedIds = new Set<number>();
  visibleIds.forEach((pageId) => {
    protectedIds.add(pageId - 1);
    protectedIds.add(pageId);
    protectedIds.add(pageId + 1);
  });
  const candidates = cached.filter((pageView) => !protectedIds.has(pageView.id));
  const destroyCount = Math.max(0, candidates.length - Math.max(0, Math.trunc(recentPageCount)));
  const destroyed = candidates.slice(0, destroyCount);
  destroyed.forEach((pageView) => {
    pageView.destroy();
    onDestroy?.(pageView.id);
  });
  const destroyedIds = new Set(destroyed.map((pageView) => pageView.id));
  return {
    destroyedPageIds: [...destroyedIds],
    retainedPageIds: cached
      .filter((pageView) => !destroyedIds.has(pageView.id))
      .map((pageView) => pageView.id),
  };
}
