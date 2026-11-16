import {expect, test} from "@playwright/test";

declare global {interface Window {__annotationReads: string[]}}

test("a retained production PDF does not reload annotations after an unrelated note save", async ({page}) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/?preview=app");
  await page.getByRole("row", {name:/Synthetic Research Preview/}).dblclick();
  await expect(page.getByText("This PDF is a local sample for frontend interaction testing.", {exact:true})).toBeVisible();
  await page.evaluate(async () => {
    const {getNotesGateway} = await import("/ra-workbench/apps/web/src/core/features/notes/gateways/index.ts");
    const gateway = getNotesGateway(), original = gateway.listReadingNotes.bind(gateway);
    window.__annotationReads = [];
    gateway.listReadingNotes = async query => {
      if (query?.paperId) window.__annotationReads.push(`${query.paperId}:${query.status}`);
      return original(query);
    };
  });
  await page.getByRole("button", {name:"笔记",exact:true}).click();
  const note = page.locator(".notes-explorer").getByRole("link", {name:/打开笔记：研究计划/}).first();
  await note.click();
  const editor = page.locator(".retained-note-editor:not([hidden]) .bn-editor");
  await expect(editor).toBeVisible();
  await page.evaluate(() => {window.__annotationReads = [];});
  await editor.click(); await page.keyboard.press("ControlOrMeta+End");
  await page.keyboard.insertText(" annotation refresh regression");
  await expect(editor).toContainText("annotation refresh regression");
  await expect(page.locator(".notes-content:not([hidden]) .note-save-state")).toHaveAttribute("aria-label","已保存");
  expect(await page.evaluate(() => window.__annotationReads)).toEqual([]);
  await page.getByRole("button", {name:"文献",exact:true}).click();
  await expect(page.getByText("This PDF is a local sample for frontend interaction testing.", {exact:true})).toBeVisible();
  expect(await page.evaluate(() => window.__annotationReads)).toEqual([]);
  expect(errors).toEqual([]);
  await page.screenshot({path:".tmp/reader-annotation-refresh.png"});
});
