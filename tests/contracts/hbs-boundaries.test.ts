import { readFileSync, readdirSync } from "node:fs";
import { expect, it } from "vitest";

it("keeps HBS components independent of the experiment", () => {
  const path = "apps/web/src/core/components/hbs";
  for (const name of readdirSync(path).filter(name => !name.includes(".test."))) {
    expect(readFileSync(`${path}/${name}`, "utf8")).not.toMatch(/from ["'][^"']*experiments/u);
  }
});

it("prevents business hosts from importing the frozen experiment", () => {
  const visit = (directory: string) => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = `${directory}/${entry.name}`;
      if (entry.isDirectory()) { if (entry.name !== "experiments") visit(path); }
      else if (/\.tsx?$/u.test(entry.name) && !entry.name.includes(".test.")) {
        expect(readFileSync(path, "utf8"), path).not.toMatch(/(?:from\s*|import\s*\()\s*["'][^"']*(?:experiments\/sync-model|references\/projects\/bowen-xu-hbs-sync-model)/u);
      }
    }
  };
  visit("apps/web/src");
});
