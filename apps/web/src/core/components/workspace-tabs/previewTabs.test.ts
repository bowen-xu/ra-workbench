import { describe, expect, it } from "vitest";
import { openPreviewTab } from "./previewTabs";

describe("preview tab lifecycle", () => {
  it("replaces only the transient note and retains fixed notes", () => {
    const fixed = openPreviewTab([], "a", "A", true);
    const preview = openPreviewTab(fixed, "b", "B");
    expect(openPreviewTab(preview, "c", "C")).toEqual([
      { id: "a", title: "A", preview: false }, { id: "c", title: "C", preview: true },
    ]);
  });
  it("promotes the same note on double click without duplication or later demotion", () => {
    const preview = openPreviewTab([], "a", "A");
    const pinned = openPreviewTab(preview, "a", "A", true);
    expect(openPreviewTab(pinned, "a", "Renamed")).toEqual([{ id: "a", title: "Renamed", preview: false }]);
    expect(openPreviewTab(pinned, "b", "B")).toHaveLength(2);
  });
});
