import {projectPath} from "./projectPaths";
import {test,expect} from "@playwright/test";
import {resolve} from "node:path";
for (const entry of ["verifyHeaderWrites","verifyHeaderUpgrade"] as const) test(entry,async ({page}) => {
  await page.goto("/@vite/client");
  const result = await page.evaluate(async ({file,entry}) => (await import("/@fs/"+file))[entry](),{file: projectPath("tests/ui/note-header-writes.browser.ts"),entry});
  if (entry === "verifyHeaderWrites") expect(result).toMatchObject({cold: {headerWrites: 0,canonicalReads: 1},version: 10,rolledBack: true});
  else expect(result).toMatchObject({closed: true,rejected: true,upgradeReads: 0,upgradeWrites: 0,version: 17});
  console.log("note-header-writes",JSON.stringify(result));
});
