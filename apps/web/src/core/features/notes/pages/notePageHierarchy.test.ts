import { describe, expect, it } from "vitest";

import { notePageId, type NotePage } from "@research-assistant/core/notes";

import { descendantPageIds, siblingCount } from "./notePageHierarchy";

function page(id: string, parent?: string): NotePage {
  return {
    id: notePageId(id), title: id, ...(parent === undefined ? {} : { parentPageId: notePageId(parent) }),
    position: 0, status: "active", isFavorite: false, revision: 0, schemaVersion: 1,
    projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now",
  };
}

describe("note page hierarchy", () => {
  it("finds all descendants and counts only siblings", () => {
    const root = page("root");
    const child = page("child", "root");
    const pages = [root, child, page("grandchild", "child"), page("other")];
    expect([...descendantPageIds(pages, notePageId("root"))]).toEqual([notePageId("child"), notePageId("grandchild")]);
    expect(siblingCount(pages, root)).toBe(2);
    expect(siblingCount(pages, child)).toBe(1);
  });
});
