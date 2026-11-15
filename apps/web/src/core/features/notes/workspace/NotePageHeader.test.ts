import { describe, expect, it } from "vitest";
import { notePageId, type NotePage } from "@research-assistant/core/notes";

import { notePageBreadcrumb } from "./NotePageHeader";

function page(id: string, parentId?: string): NotePage {
  return { id: notePageId(id), title: id, position: 0, status: "active", isFavorite: false, revision: 1, schemaVersion: 1, projectIds: [], tagIds: [], createdAt: "now", updatedAt: "now", ...(parentId === undefined ? {} : { parentPageId: notePageId(parentId) }) };
}

describe("NotePageHeader", () => {
  it("builds a root-to-parent breadcrumb", () => {
    const pages = [page("root"), page("middle", "root"), page("leaf", "middle")];
    expect(notePageBreadcrumb(pages, pages[2]!).map((item) => item.id)).toEqual([notePageId("root"), notePageId("middle")]);
  });
});
