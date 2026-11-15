import { afterEach, expect, it, vi } from "vitest";
import { notePageId } from "@research-assistant/core/notes";
import type { NotesGateway } from "@research-assistant/data/notes";
import { mockNoteOpening, readNoteOpeningScenario } from "./mockNoteOpening";

const id = notePageId("preview-page");
afterEach(() => vi.useRealTimers());
function fixture(scenario: "missing" | "failed" | "loading") {
  const load = vi.fn(async () => ({ page: {id} }));
  return { load, gateway: mockNoteOpening({loadDocument: load} as unknown as NotesGateway, scenario, id) };
}
it("scopes fault scenarios to the isolated notes preview", () => {
  expect(readNoteOpeningScenario("?preview=notes&note-open=missing")).toBe("missing");
  expect(readNoteOpeningScenario("?preview=app&note-open=missing")).toBeUndefined();
  expect(readNoteOpeningScenario("?note-open=failed")).toBeUndefined();
  expect(readNoteOpeningScenario("?preview=notes&note-open=unknown")).toBeUndefined();
});
it("keeps missing pages missing and leaves unrelated reads alone", async () => {
  const f = fixture("missing");
  await expect(f.gateway.loadDocument(id)).rejects.toMatchObject({code: "not_found"});
  await expect(f.gateway.loadDocument(id)).rejects.toMatchObject({code: "not_found"});
  await f.gateway.loadDocument(notePageId("another-page"));
  expect(f.load).toHaveBeenCalledExactlyOnceWith("another-page");
});
it("recovers on retry after the failed read", async () => {
  const f = fixture("failed");
  await expect(f.gateway.loadDocument(id)).rejects.toMatchObject({code: "database_busy", message: expect.stringContaining("operation=note-open-preview")});
  await f.gateway.loadDocument(id);
  expect(f.load).toHaveBeenCalledOnce();
});
it("delays only the first loading read", async () => {
  vi.useFakeTimers(); const f = fixture("loading"), pending = f.gateway.loadDocument(id);
  await vi.advanceTimersByTimeAsync(7_999); expect(f.load).not.toHaveBeenCalled();
  await vi.advanceTimersByTimeAsync(1); await pending;
  await f.gateway.loadDocument(id); expect(f.load).toHaveBeenCalledTimes(2);
});
