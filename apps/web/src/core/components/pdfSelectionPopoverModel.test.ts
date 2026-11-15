import { describe, expect, it } from "vitest";

import { positionPdfSelectionPopover } from "./pdfSelectionPopoverModel";

const boundary = { left: 100, top: 50, right: 900, bottom: 650 };
const popup = { width: 320, height: 180 };

describe("PDF selection popover positioning", () => {
  it("keeps the note action under the right-click point when its color picker extends to the right", () => {
    expect(positionPdfSelectionPopover({ anchor: { left: 400, top: 180, right: 400.01, bottom: 180.01 }, boundary,
      popup: { width: 68, height: 38 }, centerOnAnchor: true, anchorControlCenter: 25 })).toEqual({ left: 375, top: 161, placement: "center" });
  });
  it("places the explanation icon at the clicked point with the note icon to its right", () => {
    expect(positionPdfSelectionPopover({ anchor: { left: 400, top: 180, right: 400, bottom: 180 }, boundary,
      popup: { width: 94, height: 38 }, centerOnAnchor: true, anchorControlCenter: 47 })).toEqual({ left: 353, top: 161, placement: "center" });
  });
  it("centers a position-note action on the context-menu point", () => {
    expect(positionPdfSelectionPopover({
      anchor: { left: 400, top: 180, right: 400.01, bottom: 180.01 },
      boundary,
      popup: { width: 38, height: 38 },
      centerOnAnchor: true,
    })).toEqual({ left: 381, top: 161, placement: "center" });
  });

  it("only shifts a centered point action when it would cross a reading boundary", () => {
    const pointPopup = { width: 38, height: 38 };
    expect(positionPdfSelectionPopover({
      anchor: { left: 102, top: 52, right: 102.01, bottom: 52.01 },
      boundary,
      popup: pointPopup,
      centerOnAnchor: true,
    })).toEqual({ left: 110, top: 60, placement: "center" });
    expect(positionPdfSelectionPopover({
      anchor: { left: 898, top: 648, right: 898.01, bottom: 648.01 },
      boundary,
      popup: pointPopup,
      centerOnAnchor: true,
    })).toEqual({ left: 852, top: 602, placement: "center" });
  });

  it("places the popover below the selection when the lower side fits", () => {
    expect(positionPdfSelectionPopover({
      anchor: { left: 400, top: 180, right: 460, bottom: 200 },
      boundary,
      popup,
    })).toEqual({ left: 270, top: 208, placement: "below" });
  });

  it("flips above the selection before it would cross the lower boundary", () => {
    expect(positionPdfSelectionPopover({
      anchor: { left: 400, top: 570, right: 460, bottom: 590 },
      boundary,
      popup,
    })).toEqual({ left: 270, top: 382, placement: "above" });
  });

  it("follows the selection while keeping the popover pinned to viewport edges", () => {
    expect(positionPdfSelectionPopover({
      anchor: { left: 400, top: -80, right: 460, bottom: -60 },
      boundary,
      popup,
    })).toEqual({ left: 270, top: 60, placement: "below" });

    expect(positionPdfSelectionPopover({
      anchor: { left: 400, top: 720, right: 460, bottom: 740 },
      boundary,
      popup,
    })).toEqual({ left: 270, top: 460, placement: "above" });
  });

  it("keeps the popover inside the horizontal reading boundary", () => {
    expect(positionPdfSelectionPopover({
      anchor: { left: 70, top: 180, right: 90, bottom: 200 },
      boundary,
      popup,
    }).left).toBe(110);
    expect(positionPdfSelectionPopover({
      anchor: { left: 930, top: 180, right: 950, bottom: 200 },
      boundary,
      popup,
    }).left).toBe(570);
  });
});
