import { expect, it } from "vitest";
import { selectAppStorage } from "../../../../../../apps/web/src/core/runtime/appStorage";
import { moveLiteratureNote, orderedLiteratureNoteIds, readLiteratureNoteOrder, saveLiteratureNoteOrder } from "../../../../../../../plugins/ra-literature-library/src/features/reader/literatureNoteOrder";

it("preserves manual order across reopening, prepends new notes and isolates papers", () => {
  const data = new Map<string, string>();
  selectAppStorage({ get length() { return data.size; }, key: index => [...data.keys()][index] ?? null,
    getItem: key => data.get(key) ?? null, setItem: (key, value) => { data.set(key, value); }, removeItem: key => { data.delete(key); }, clear: () => data.clear() });
  const moved = moveLiteratureNote(["new", "middle", "old"], "old", "new");
  expect(moved).toEqual(["old", "new", "middle"]);
  saveLiteratureNoteOrder("paper", moved);
  expect(orderedLiteratureNoteIds(["newest", "new", "middle", "old"], readLiteratureNoteOrder("paper"))).toEqual(["newest", "old", "new", "middle"]);
  expect(orderedLiteratureNoteIds(["new", "middle"], readLiteratureNoteOrder("paper"))).toEqual(["new", "middle"]);
  expect(readLiteratureNoteOrder("other")).toEqual([]);
  expect(moveLiteratureNote(moved, "missing", "new")).toBe(moved);
});
