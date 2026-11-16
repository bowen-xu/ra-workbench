import {projectPath} from "./projectPaths";
import {test,expect} from "@playwright/test";
import {resolve} from "node:path";

test("preparation reads only requested lifecycle bodies and reading titles",async ({page}) => {
  await page.goto("/@vite/client");
  const result = await page.evaluate(async path => (await import("/@fs/" + path)).verifyCatalogScopes(),projectPath("tests/ui/note-catalog-scopes.browser.ts"));
  expect(result).toEqual({firstListBodies: 0,activeTitleBodies: 2,activeSearchBodies: 16,liveSearchBodies: 32,hotOrReopenBodies: 0,allScopesBodies: 48});
  console.log("note-catalog-scopes",JSON.stringify(result));
});

for (const duringOtherScope of [false,true]) test(`status move cannot hide an unindexed identity behind a cursor: concurrent=${duringOtherScope}`,async ({page}) => {
  await page.goto("/@vite/client");
  const result = await page.evaluate(async ({path,duringOtherScope}) => (await import("/@fs/" + path)).verifyCatalogScopeMoves(duringOtherScope),{path: projectPath("tests/ui/note-catalog-scopes.browser.ts"),duringOtherScope});
  expect(result).toEqual({duringOtherScope,bodyReads: duringOtherScope ? 26 : 25,movedReads: 1,version: duringOtherScope ? 3 : 2});
  console.log("note-catalog-scope-move",JSON.stringify(result));
});

for (const complete of [true,false]) test(`v7 readiness and bodies survive the metadata-only range-index upgrade: complete=${complete}`,async ({page}) => {
  await page.goto("/@vite/client");
  const result = await page.evaluate(async ({path,complete}) => (await import("/@fs/" + path)).verifyCatalogScopeUpgrade(complete),{path: projectPath("tests/ui/note-catalog-scopes.browser.ts"),complete});
  expect(result).toEqual({closed: true,upgradeReads: 0,bodyReads: complete ? 0 : 1,version: 2});
  console.log("note-catalog-scoped-upgrade",JSON.stringify(result));
});
