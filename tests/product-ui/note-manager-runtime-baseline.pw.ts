import {expect, test} from "@playwright/test";

test("note management preserves the same reading note through editing, trash, restore and durable search", async ({page}) => {
  await page.goto("/?preview=app");
  await expect(page.getByRole("button", {name: "笔记", exact: true})).toBeVisible();
  const result = await page.evaluate(async () => {
    const module = await import("/integration/web/src/app/notes/NoteManagerContinuity.browser.tsx");
    return module.verifyReadingNoteSearchWorkflow();
  });
  expect(result.edited).toMatchObject({id: result.id, pageId: result.pageId, body: "revised"});
  expect(result.restored).toMatchObject({id: result.id, pageId: result.pageId, body: "revised", status: "active"});
  expect(result.active).toEqual([result.id]);
  expect(result.whileTrashed).toEqual([]);
  expect(result.afterRestore).toEqual([result.id]);
  expect(result.reopenedDocument).toEqual(result.savedDocument);
});
