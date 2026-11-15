// @vitest-environment happy-dom
import { expect, it, vi } from "vitest";
import { createMockBoundResourceReader } from "./mockBoundResourceAvailable";
import { emptyLiteratureDraft } from "@research-assistant/core/literature";

it("selects the actual resource data port for each kind, without substituting a reading page for its source block", async () => {
  const writing = {listManuscripts: vi.fn(async () => [])}, literature = {get: vi.fn(async () => {
    return {...emptyLiteratureDraft(), id: "paper", attachments: [], projectIds: [], metadataSources: [], readingStatus: "unread" as const, createdAt: "now", updatedAt: "now"};
  })};
  const notes = vi.fn(async (kind: "note" | "readingNote", id: string) => kind === "readingNote" && id === "source-block");
  const available = createMockBoundResourceReader(writing, literature, notes);
  expect(await available({kind: "manuscript", resourceId: "missing"})).toBe(false);
  expect(await available({kind: "paper", resourceId: "paper"})).toBe(true);
  expect(await available({kind: "readingNote", resourceId: "source-block"})).toBe(true);
  expect(await available({kind: "note", resourceId: "source-block"})).toBe(false);
  expect(await available({kind: "readingNote", resourceId: "reading-page"})).toBe(false);
  expect(notes.mock.calls).toEqual([["readingNote", "source-block"], ["note", "source-block"], ["readingNote", "reading-page"]]);
  expect(writing.listManuscripts).toHaveBeenCalledTimes(1); expect(literature.get).toHaveBeenCalledExactlyOnceWith("paper");
});
it("treats only the browser metadata adapter's missing record as absent, preserving other diagnostics", async () => {
  const missing = new Error("找不到文献"), failed = new Error("metadata store unavailable");
  const literature = {get: vi.fn(async (): Promise<never> => { throw missing; })};
  const available = createMockBoundResourceReader({listManuscripts: async () => []}, literature);
  expect(await available({kind: "paper", resourceId: "missing"})).toBe(false);
  literature.get.mockRejectedValueOnce(failed);
  await expect(available({kind: "paper", resourceId: "missing"})).rejects.toBe(failed);
});
