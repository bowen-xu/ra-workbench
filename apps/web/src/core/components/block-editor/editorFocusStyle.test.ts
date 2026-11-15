import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

describe("note editor focus styling", () => {
  it("uses the caret instead of the global full-editor focus ring", () => {
    const styles = readFileSync(
      resolve(process.cwd(), "apps/web/src/core/components/block-editor/BlockEditor.css"),
      "utf8",
    );

    expect(styles).toMatch(
      /\.block-editor \.bn-editor:focus-visible \{[^}]*outline: 0 !important;[^}]*box-shadow: none !important;[^}]*caret-color: #245d3d;[^}]*\}/u,
    );
  });
});
