import { describe, expect, it } from "vitest";

import { blockDragScrollVelocity } from "./useBlockDragAutoScroll";

describe("block drag auto scroll", () => {
  it("scrolls only inside the 48px edge zones", () => {
    expect(blockDragScrollVelocity(100, 100, 500)).toBe(-16);
    expect(blockDragScrollVelocity(148, 100, 500)).toBe(0);
    expect(blockDragScrollVelocity(300, 100, 500)).toBe(0);
    expect(blockDragScrollVelocity(500, 100, 500)).toBe(16);
  });
});
