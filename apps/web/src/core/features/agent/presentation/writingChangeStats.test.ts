import { expect, it } from "vitest";
import { countWritingChangedLines } from "./writingChangeStats";

it.each([
  ["unchanged", "unchanged\nnew", 1, 0],
  ["unchanged\n", "unchanged\nnew\n", 1, 0],
  ["old\ncontext\nlast", "new\ncontext\nchanged", 2, 2],
  ["a\na\nb", "a\nb", 0, 1],
  ["", "one\ntwo\n", 2, 0],
  ["one\ntwo\n", "", 0, 2],
  ["", "\n", 1, 0],
  ["\n", "", 0, 1],
  ["a\nb", "a\n\nb", 1, 0],
] as const)("counts real line changes %j → %j without unchanged context", (before, after, additions, deletions) => {
  expect(countWritingChangedLines(before, after)).toEqual({ additions, deletions });
});

it("preserves line identity when a long review uses supplementary code points", () => {
  const lines = Array.from({ length: 20_000 }, (_, index) => `line ${index} 😀`);
  const changed = [...lines]; changed[19_000] = "changed";
  expect(countWritingChangedLines(lines.join("\n"), changed.join("\n"))).toEqual({ additions: 1, deletions: 1 });
});
