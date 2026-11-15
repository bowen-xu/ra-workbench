import { describe, expect, it } from "vitest";
import { getSelectedBlockRootIds } from "./blockSelectionRoots";

const tree = [
  { id: "parent", children: [
    { id: "child-a", children: [{ id: "grandchild", children: [] }] },
    { id: "child-b", children: [] },
  ] },
  { id: "tail", children: [] },
];

describe("block selection roots", () => {
  it("keeps document order, removes duplicates and never includes a selected descendant twice", () => {
    expect(getSelectedBlockRootIds(tree, ["tail", "grandchild", "parent", "child-b", "tail"]))
      .toEqual(["parent", "tail"]);
  });
  it("selects children independently without silently selecting their ancestor", () => {
    expect(getSelectedBlockRootIds(tree, ["tail", "child-b", "grandchild"]))
      .toEqual(["grandchild", "child-b", "tail"]);
  });
  it("omits absent identities and supports an empty selection", () => {
    expect(getSelectedBlockRootIds(tree, ["deleted"])).toEqual([]);
    expect(getSelectedBlockRootIds(tree, [])).toEqual([]);
  });
});
