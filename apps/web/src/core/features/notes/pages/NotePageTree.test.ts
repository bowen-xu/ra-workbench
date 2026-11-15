import { describe, expect, it } from "vitest";
import { notePageId, type NotePage } from "@research-assistant/core/notes";

import { pageDropDestination, pageDropZone, pageTreeKeyboardTarget } from "./NotePageTree";

function page(id: string, position: number, parentId?: string): NotePage {
  return {
    id: notePageId(id), title: id, position, status: "active", isFavorite: false,
    revision: 1, schemaVersion: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now",
    ...(parentId === undefined ? {} : { parentPageId: notePageId(parentId) }),
  };
}

describe("NotePageTree keyboard navigation", () => {
  it("moves through the visible tree without leaving its bounds", () => {
    expect(pageTreeKeyboardTarget(1, 4, "ArrowDown")).toBe(2);
    expect(pageTreeKeyboardTarget(3, 4, "ArrowDown")).toBe(3);
    expect(pageTreeKeyboardTarget(1, 4, "ArrowUp")).toBe(0);
    expect(pageTreeKeyboardTarget(0, 4, "ArrowUp")).toBe(0);
    expect(pageTreeKeyboardTarget(2, 4, "Home")).toBe(0);
    expect(pageTreeKeyboardTarget(1, 4, "End")).toBe(3);
    expect(pageTreeKeyboardTarget(1, 4, "Enter")).toBeUndefined();
  });
});

describe("NotePageTree drop targets", () => {
  it("uses 25/50/25 hit zones", () => {
    expect(pageDropZone(100, 100, 40)).toBe("before");
    expect(pageDropZone(110, 100, 40)).toBe("inside");
    expect(pageDropZone(139, 100, 40)).toBe("after");
  });

  it("corrects positions when reordering within one parent", () => {
    const pages = [page("a", 0), page("b", 1), page("c", 2)];
    expect(pageDropDestination(pages, notePageId("a"), notePageId("c"), "after")).toEqual({ parentId: undefined, position: 2 });
    expect(pageDropDestination(pages, notePageId("c"), notePageId("a"), "before")).toEqual({ parentId: undefined, position: 0 });
  });

  it("moves across parents and appends on an inside drop", () => {
    const pages = [page("a", 0), page("b", 1), page("child", 0, "b")];
    expect(pageDropDestination(pages, notePageId("a"), notePageId("b"), "inside")).toEqual({ parentId: notePageId("b"), position: 1 });
  });

  it("rejects moving a page into itself or a descendant", () => {
    const pages = [page("a", 0), page("b", 0, "a"), page("c", 0, "b")];
    expect(pageDropDestination(pages, notePageId("a"), notePageId("a"), "inside")).toBeUndefined();
    expect(pageDropDestination(pages, notePageId("a"), notePageId("c"), "inside")).toBeUndefined();
  });
});
