export interface PdfSelectionPopoverRect {
  readonly left: number;
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
}

export interface PdfSelectionPopoverSize {
  readonly width: number;
  readonly height: number;
}

export interface PdfSelectionPopoverPosition {
  readonly left: number;
  readonly top: number;
  readonly placement: "above" | "below" | "center";
}

interface PdfSelectionPopoverPositionInput {
  readonly anchor: PdfSelectionPopoverRect;
  readonly boundary: PdfSelectionPopoverRect;
  readonly popup: PdfSelectionPopoverSize;
  readonly gap?: number;
  readonly padding?: number;
  readonly centerOnAnchor?: boolean;
  readonly anchorControlCenter?: number;
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(Math.max(value, minimum), Math.max(minimum, maximum));
}

export function positionPdfSelectionPopover({
  anchor,
  boundary,
  popup,
  gap = 8,
  padding = 10,
  centerOnAnchor = false,
  anchorControlCenter,
}: PdfSelectionPopoverPositionInput): PdfSelectionPopoverPosition {
  const minimumLeft = boundary.left + padding;
  const maximumLeft = boundary.right - padding - popup.width;
  const minimumTop = boundary.top + padding;
  const maximumTop = boundary.bottom - padding - popup.height;
  if (centerOnAnchor) {
    return {
      left: clamp(anchor.left - (anchorControlCenter ?? popup.width / 2), minimumLeft, maximumLeft),
      top: clamp(anchor.top - popup.height / 2, minimumTop, maximumTop),
      placement: "center",
    };
  }
  const fitsBelow = anchor.bottom + gap + popup.height <= boundary.bottom - padding;
  const placement = fitsBelow ? "below" : "above";
  const idealTop = placement === "below"
    ? anchor.bottom + gap
    : anchor.top - gap - popup.height;
  const anchorCenter = anchor.left + (anchor.right - anchor.left) / 2;

  return {
    left: clamp(anchorCenter - popup.width / 2, minimumLeft, maximumLeft),
    top: clamp(idealTop, minimumTop, maximumTop),
    placement,
  };
}
