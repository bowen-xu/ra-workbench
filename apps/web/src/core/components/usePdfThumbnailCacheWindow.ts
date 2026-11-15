import { useEffect, useMemo, useState, type RefObject } from "react";

const PDF_THUMBNAIL_CACHE_LIMIT = 8;

/** Keep at most twice the viewport's displayed thumbnails, including partial pages. */
export function usePdfThumbnailCacheWindow(
  options: {
    readonly listRef: RefObject<HTMLDivElement | null>;
    readonly pageCount: number;
    readonly currentPageNumber: number;
    readonly renderingEnabled: boolean;
  },
): ReadonlySet<number> {
  const { listRef, pageCount, currentPageNumber, renderingEnabled } = options;
  const [visiblePages, setVisiblePages] = useState<readonly number[]>([]);

  useEffect(() => {
    const list = listRef.current;
    if (list === null || !renderingEnabled || typeof IntersectionObserver === "undefined") return;
    const pages = new Map<Element, number>();
    const visible = new Set<number>();
    let disconnected = false;
    const observer = new IntersectionObserver((entries) => {
      if (disconnected) return;
      for (const entry of entries) {
        const page = pages.get(entry.target);
        if (page === undefined) continue;
        if (entry.isIntersecting && entry.intersectionRect.height > 0) visible.add(page);
        else visible.delete(page);
      }
      const next = [...visible].sort((left, right) => left - right);
      setVisiblePages(previous => previous.length === next.length && previous.every((page, index) => page === next[index]) ? previous : next);
    }, { root: list.closest(".reader-navigation"), rootMargin: "0px" });
    Array.from(list.children).forEach((element, index) => {
      pages.set(element, index + 1);
      observer.observe(element);
    });
    return () => {
      disconnected = true;
      observer.disconnect();
    };
  }, [listRef, pageCount, renderingEnabled]);

  return useMemo(() => {
    if (typeof IntersectionObserver === "undefined") return new Set([currentPageNumber]);
    // Preserve the bounded window while hidden so tab activation alone need not repaint it.
    const pages = visiblePages.filter(page => page <= pageCount);
    const prioritized = pages.includes(currentPageNumber)
      ? [currentPageNumber, ...pages.filter(page => page !== currentPageNumber)]
      : pages;
    const budget = Math.min(PDF_THUMBNAIL_CACHE_LIMIT, pages.length * 2);
    const retained = new Set(prioritized.slice(0, budget));
    const first = pages[0];
    const last = pages.at(-1);
    if (first === undefined || last === undefined) return retained;
    // Spare slots keep nearby pages warm without displacing any visible bitmap.
    for (let distance = 1; retained.size < budget && (first - distance >= 1 || last + distance <= pageCount); distance += 1) {
      if (first - distance >= 1) retained.add(first - distance);
      if (retained.size < budget && last + distance <= pageCount) retained.add(last + distance);
    }
    return retained;
  }, [currentPageNumber, pageCount, visiblePages]);
}
