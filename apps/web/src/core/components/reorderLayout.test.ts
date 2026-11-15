import { expect, it } from "vitest";
import { reorderedRowIds, reorderRowOffsets } from "./reorderLayout";

it("keeps variable-height rows contiguous when moving in either direction", () => {
  const rows = [{ id: "short", top: 0, height: 20 }, { id: "tall", top: 24, height: 60 }, { id: "last", top: 88, height: 30 }];
  const down = reorderedRowIds(rows.map(row => row.id), ["short"], "last", "after");
  expect(down).toEqual(["tall", "last", "short"]);
  expect([...reorderRowOffsets(rows, down)]).toEqual([["tall", -24], ["last", -24], ["short", 98]]);
  const up = reorderedRowIds(rows.map(row => row.id), ["last"], "short", "before");
  expect([...reorderRowOffsets(rows, up)]).toEqual([["last", -88], ["short", 34], ["tall", 34]]);
});
it("preserves document order for nonadjacent selected rows and ignores self targets", () => {
  const ids = ["a", "b", "c", "d"];
  expect(reorderedRowIds(ids, ["c", "a"], "d", "after")).toEqual(["b", "d", "a", "c"]);
  expect(reorderedRowIds(ids, ["a", "c"], "c", "before")).toBe(ids);
});
it("can lay out a new parent group using explicit origin and gap", () => {
  const rows = [{ id: "nested", top: 40, height: 20 }, { id: "peer", top: 100, height: 30 }];
  expect([...reorderRowOffsets(rows, ["peer", "nested"], { top: 200, gap: 4 })]).toEqual([["peer", 100], ["nested", 194]]);
});
