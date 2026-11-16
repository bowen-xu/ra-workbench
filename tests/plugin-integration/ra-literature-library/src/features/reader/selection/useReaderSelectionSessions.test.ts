import { describe, expect, it } from "vitest";

import type { PdfCharacterSelection } from "../../../../../../../apps/web/src/core/components/pdf-view/pdfPageCharacterModel";
import { createPdfSelectionSession } from "../../../../../../../../plugins/ra-literature-library/src/features/reader/selection/useReaderSelectionSessions";

const firstRect = { x: 0.1, y: 0.2, width: 0.3, height: 0.04 };
const focusRect = { x: 0.2, y: 0.7, width: 0.25, height: 0.05 };

function selection(): PdfCharacterSelection {
  return {
    pageIndex: 0,
    start: 1,
    end: 12,
    text: "selected evidence",
    prefix: "before",
    suffix: "after second page",
    rects: [firstRect],
    lastClientRect: { left: 10, top: 20, right: 40, bottom: 24, width: 30, height: 4 },
    focusPageIndex: 1,
    focusRect,
    segments: [
      {
        pageIndex: 0,
        start: 1,
        end: 8,
        text: "selected",
        prefix: "before",
        suffix: "",
        rects: [firstRect],
        lastClientRect: { left: 10, top: 20, right: 40, bottom: 24, width: 30, height: 4 },
      },
      {
        pageIndex: 1,
        start: 0,
        end: 8,
        text: "evidence",
        prefix: "",
        suffix: "after second page",
        rects: [focusRect],
        lastClientRect: { left: 20, top: 70, right: 45, bottom: 75, width: 25, height: 5 },
      },
    ],
  };
}

describe("reader selection session", () => {
  it("turns a completed glyph selection into an active popup session", () => {
    const session = createPdfSelectionSession(selection(), ["i", "1"], "selection-session");

    expect(session).toMatchObject({
      id: "selection-session",
      active: true,
      dirty: false,
      pending: {
        text: "selected evidence",
        pageIndex: 0,
        pageLabel: "i",
        popupPageIndex: 1,
        popupRect: focusRect,
        suffix: "after second page",
      },
    });
    expect(session?.pending.segments.map((segment) => [segment.pageIndex, segment.pageLabel]))
      .toEqual([[0, "i"], [1, "1"]]);
  });

  it("does not create a popup session for an empty structured selection", () => {
    expect(createPdfSelectionSession({ ...selection(), segments: [] }, [], "empty"))
      .toBeUndefined();
  });
});
