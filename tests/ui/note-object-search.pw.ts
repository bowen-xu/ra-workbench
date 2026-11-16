import {projectPath} from "./projectPaths";
import {test,expect} from "@playwright/test";
import {resolve} from "node:path";

test.beforeEach(({page}) => {page.on("console",message => {if (message.text().startsWith("[note-object-stage]")) console.log(message.text());});});

test("existing note search uses persistent bounded queries and metadata-only upgrades",async ({page}) => {
  test.setTimeout(180_000);
  await page.goto("/@vite/client");
  const result = await page.evaluate(async path => {
    const {verifyNoteObjectSearch} = await import("/@fs/" + path);
    return verifyNoteObjectSearch();
  },projectPath("tests/ui/note-object-search.browser.ts"));
  expect(result.count).toBe(2400);
  console.log("note-object-search-metrics",JSON.stringify(result));
});

test("4000 ordinary and reading notes keep the frozen persistent-search budget",async ({page}) => {
  test.setTimeout(360_000); // Includes one-time preparation; the hot P95 gate remains 200 ms.
  await page.goto("/@vite/client");
  const result = await page.evaluate(async path => {
    const {verifyDenseNoteObjectSearch} = await import("/@fs/" + path);
    return verifyDenseNoteObjectSearch();
  },projectPath("tests/ui/note-object-search.browser.ts"));
  expect(result.count).toBe(4000);
  expect(result.p95).toBeLessThan(200);
  expect(result.metrics.rows.documents ?? 0).toBe(0);
  expect(result.metrics.rows.catalogBodies).toBe(20);
  console.log("note-object-dense-metrics",JSON.stringify(result));
});
