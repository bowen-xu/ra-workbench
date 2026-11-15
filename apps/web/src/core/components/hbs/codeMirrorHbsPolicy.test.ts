import { expect, it } from "vitest";
import { mergeText } from "@research-assistant/core/hbs";
import { createCodeMirrorHbsPolicy } from "./codeMirrorHbsPolicy";

const base = "title\n\nvalue 80%\nmethod\nlast";
const policy = createCodeMirrorHbsPolicy();
it("allows a different real diff unit policy without changing state or storage contracts", () => {
  const local = base.replace("title", "my title"), remote = base.replace("last", "new last");
  expect(mergeText(base, local, remote, policy.calculate).some(part => part.source === "conflict")).toBe(false);
  const conservative = createCodeMirrorHbsPolicy((a, b) => a === b ? [] : [{ fromA: 0, toA: a.length, fromB: 0, toB: b.length }]);
  expect(mergeText(base, local, remote, conservative.calculate).some(part => part.source === "conflict")).toBe(true);
});
