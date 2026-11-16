import { expect, test, type Page } from "@playwright/test";

async function create(page: Page, title: string) {
  const editor = page.locator(".notes-content .bn-editor");
  const previous = await editor.count() ? await editor.elementHandle() : null;
  await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
  if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
  await page.getByRole("textbox", { name: "笔记标题" }).fill(title);
  await editor.click(); await page.keyboard.type(`Body of ${title}`);
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
}

for (const url of ["/", "/?preview=app"]) {
  test(`back and forward restore note pages and reference visits on ${url}`, async ({ page }) => {
    await page.goto(url); await page.getByRole("button", { name: "笔记", exact: true }).click();
    for (const title of ["History A", "History B", "History C"]) await create(page, title);
    const title = page.getByRole("textbox", { name: "笔记标题" });
    const back = page.getByRole("button", { name: "后退", exact: true });
    const forward = page.getByRole("button", { name: "前进", exact: true });
    await back.click(); await expect(title).toHaveValue("History B");
    await expect(page.locator(".notes-content .bn-editor")).toContainText("Body of History B");
    await back.click(); await expect(title).toHaveValue("History A");
    await forward.click(); await expect(title).toHaveValue("History B");
    await page.getByRole("tab", { name: "页面", exact: true }).click();
    await page.getByRole("tree").getByRole("button", { name: "History A", exact: true }).click();
    await expect(title).toHaveValue("History A"); await expect(forward).toBeDisabled();
    const editor = page.locator(".notes-content .bn-editor");
    await editor.click(); await page.keyboard.press("End"); await page.keyboard.type(" @History B");
    await page.keyboard.press("Tab");
    await editor.getByRole("link", { name: "History B", exact: true }).click();
    await expect(title).toHaveValue("History B");
    await back.click(); await expect(title).toHaveValue("History A");
    await expect(editor.getByRole("link", { name: "History B", exact: true })).toBeVisible();
    await forward.click(); await expect(title).toHaveValue("History B");
    await page.getByRole("button", { name: "文献", exact: true }).click();
    await expect(title).toHaveCount(0);
    await back.click(); await expect(title).toHaveValue("History B");
    await forward.click(); await expect(title).toHaveCount(0);
  });
}
