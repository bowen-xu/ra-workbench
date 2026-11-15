import { expect, it } from "vitest";
import { preserveReplacementCase } from "./editorSearchBehavior";
it.each([
  ["ABC", "Def", "DEF"], ["abc", "Def", "def"], ["AbC", "DeF", "DeF"],
  ["aBc", "DeF", "deF"], ["Foo-Bar", "newfoo-newbar", "Newfoo-Newbar"],
  ["foo-BAR", "newfoo-newbar", "newfoo-NEWBAR"], ["Foo_Bar", "newfoo_newbar", "Newfoo_Newbar"],
  ["", "DeF", "DeF"], ["中文", "中文", "中文"],
])("preserves case for %s", (before, replacement, expected) => {
  expect(preserveReplacementCase(replacement, before)).toBe(expected);
});
