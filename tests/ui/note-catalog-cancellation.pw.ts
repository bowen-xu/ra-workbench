import {projectPath} from "./projectPaths";
import {test,expect} from "@playwright/test";
import {resolve} from "node:path";

for (const sharedConsumer of [false,true]) test(`browser preparation pauses only after the last consumer leaves: shared=${sharedConsumer}`,async ({page}) => {
  await page.goto("/@vite/client");
  const result = await page.evaluate(async ({path,sharedConsumer}) => {
    const {verifyCatalogCancellation} = await import("/@fs/" + path);
    return verifyCatalogCancellation(sharedConsumer);
  },{path: projectPath("tests/ui/note-catalog-cancellation.browser.ts"),sharedConsumer});
  expect(result).toMatchObject({sharedConsumer,totalBodyReads: 48,repeatReads: 0,version: sharedConsumer ? 1 : 2});
  console.log("note-catalog-cancellation",JSON.stringify(result));
});
