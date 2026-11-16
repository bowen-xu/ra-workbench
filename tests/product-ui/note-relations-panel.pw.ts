import { expect, test, type Page } from "@playwright/test";

async function createNote(page: Page, title: string) {
  const editor = page.locator(".notes-content .bn-editor");
  const previous = await editor.count() ? await editor.elementHandle() : null;
  await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
  if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
  await page.getByRole("textbox", { name: "笔记标题" }).fill(title);
  await editor.click();
  await expect(page.getByRole("tree").getByRole("button", { name: title, exact: true })).toBeVisible();
}

async function openRelations(page: Page) {
  await page.getByRole("button", { name: "更多页面操作", exact: true }).click();
  await page.getByRole("menuitem", { name: "出链与反链", exact: true }).click();
  const panel = page.getByRole("dialog", { name: "出链与反链", exact: true });
  await expect(panel).toBeVisible();
  return panel;
}

for (const url of ["/", "/?preview=app"]) {
  test(`note relations open on demand, navigate and dismiss on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await createNote(page, "Relation target");
    await createNote(page, "Relation source");
    const editor = page.locator(".notes-content .bn-editor");
    await page.keyboard.type("@Relation target");
    await expect(page.getByRole("option", { name: /Relation target/ })).toBeVisible();
    await page.keyboard.press("Tab");
    await expect(editor.getByRole("link", { name: "Relation target", exact: true })).toBeVisible();
    await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
    await expect(page.locator(".note-relations")).toHaveCount(0);
    const originalEditor = await editor.elementHandle();
    let panel = await openRelations(page);
    await expect(panel.getByRole("button", { name: /Relation target/ })).toBeVisible();
    await expect(panel).toContainText("尚无页面引用这条笔记");
    await expect.poll(() => originalEditor!.evaluate(node => node.isConnected)).toBe(true);
    await page.keyboard.press("Escape");
    await expect(panel).toHaveCount(0);
    await expect(page.getByRole("button", { name: "更多页面操作", exact: true })).toBeFocused();
    panel = await openRelations(page);
    await panel.getByRole("button", { name: /Relation target/ }).click();
    await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("Relation target");
    await expect(page.locator(".note-relations")).toHaveCount(0);
    panel = await openRelations(page);
    await expect(panel).toContainText("尚未引用其他研究对象");
    await panel.getByRole("button", { name: /Relation source/ }).click();
    await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("Relation source");
    panel = await openRelations(page);
    await editor.click();
    await expect(panel).toBeVisible();
    await page.getByRole("button", { name: "更多页面操作", exact: true }).click();
    const historyAction = page.getByRole("menuitem", { name: "版本历史", exact: true });
    await expect.poll(() => historyAction.evaluate(element => {
      const box = element.getBoundingClientRect();
      const hit = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
      return hit !== null && element.contains(hit);
    })).toBe(true);
    await page.screenshot({ path: test.info().outputPath("note-relations-menu-paint-order.png") });
    await historyAction.click();
    await expect(panel).toHaveCount(0);
    await expect(page.getByRole("complementary", { name: "版本历史", exact: true })).toBeVisible();
    panel = await openRelations(page);
    await expect(page.getByRole("complementary", { name: "版本历史", exact: true })).toHaveCount(0);
    await page.setViewportSize({ width: 650, height: 600 });
    const bounds = await panel.boundingBox();
    expect(bounds).not.toBeNull();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(650);
    await page.screenshot({ path: test.info().outputPath("note-relations-panel.png") });
    await panel.getByRole("button", { name: "关闭出链与反链", exact: true }).click();
    await expect(panel).toHaveCount(0);
    await expect(page.getByRole("button", { name: "更多页面操作", exact: true })).toBeFocused();
  });
}
