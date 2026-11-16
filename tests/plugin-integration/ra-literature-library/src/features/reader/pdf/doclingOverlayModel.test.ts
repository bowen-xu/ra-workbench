import { describe, expect, it } from "vitest";

import {
  DoclingOverlayRequestTracker,
  doclingOverlayItemLabel,
  hitTestDoclingOverlay,
} from "../../../../../../../../plugins/ra-literature-library/src/features/reader/pdf/doclingOverlayModel";
import type { DoclingPageOverlay } from "../../../../../../../apps/web/src/core/services/doclingGateway";

const overlay: DoclingPageOverlay = {
  attachmentId: "att-1",
  parseRunId: "run-1",
  pageIndex: 0,
  pageSize: { width: 200, height: 400 },
  items: [
    {
      id: "#/texts/0",
      kind: "paragraph",
      label: "paragraph",
      textPreview: "A short block",
      rawRef: "#/texts/0",
      regions: [{ pageIndex: 0, rect: { x: 0.1, y: 0.2, width: 0.3, height: 0.1 } }],
    },
  ],
};

describe("Docling PDF overlay hit testing", () => {
  it("returns the structured item under a normalized page point", () => {
    expect(hitTestDoclingOverlay(overlay, { x: 0.2, y: 0.25 })?.item.id).toBe("#/texts/0");
    expect(hitTestDoclingOverlay(overlay, { x: 0.5, y: 0.25 })).toBeUndefined();
    expect(hitTestDoclingOverlay(overlay, { x: -0.1, y: 0.25 })).toBeUndefined();
  });

  it("uses readable labels for hover tooltips", () => {
    const firstItem = overlay.items[0];
    expect(firstItem).toBeDefined();
    expect(doclingOverlayItemLabel(firstItem!, "1")).toBe("段落 · 第 1 页 · A short block");
  });
});

describe("Docling overlay request lifecycle", () => {
  it("does not invalidate sibling page requests when one page completes first", () => {
    const tracker = new DoclingOverlayRequestTracker();

    const firstPageGeneration = tracker.startPageRequest(0, false);
    const secondPageGeneration = tracker.startPageRequest(1, false);
    const thirdPageGeneration = tracker.startPageRequest(2, false);

    expect(firstPageGeneration).toBeDefined();
    expect(secondPageGeneration).toBe(firstPageGeneration);
    expect(thirdPageGeneration).toBe(firstPageGeneration);
    expect(tracker.pendingPages()).toEqual([0, 1, 2]);

    tracker.finishPageRequest(0);

    expect(tracker.accepts(secondPageGeneration!)).toBe(true);
    expect(tracker.accepts(thirdPageGeneration!)).toBe(true);
    expect(tracker.pendingPages()).toEqual([1, 2]);
  });

  it("invalidates old page requests only when the overlay session resets", () => {
    const tracker = new DoclingOverlayRequestTracker();
    const oldGeneration = tracker.startPageRequest(0, false);

    tracker.reset();

    const newGeneration = tracker.startPageRequest(0, false);
    expect(tracker.accepts(oldGeneration!)).toBe(false);
    expect(tracker.accepts(newGeneration!)).toBe(true);
  });

  it("does not request pages that are already cached or pending", () => {
    const tracker = new DoclingOverlayRequestTracker();

    expect(tracker.startPageRequest(0, true)).toBeUndefined();
    expect(tracker.startPageRequest(1, false)).toBeDefined();
    expect(tracker.startPageRequest(1, false)).toBeUndefined();
  });
});
