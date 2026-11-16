import { expect, test } from "@playwright/test";

for (const url of ["/", "/?preview=app"]) {
  test(`selected text becomes an undoable inline formula by button or shortcut on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    const editor = page.locator(".notes-content .bn-editor");
    const previous = await editor.count() ? await editor.elementHandle() : null;
    await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
    if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
    await page.getByRole("textbox", { name: "笔记标题", exact: true }).fill("Formula selection");
    await editor.click(); await page.keyboard.type("E=mc^2");
    await page.keyboard.press("ControlOrMeta+a");
    const toolbar = page.locator(".bn-formatting-toolbar");
    const formula = toolbar.getByRole("button", { name: "标记为公式", exact: true });
    await expect(formula).toBeVisible();
    await expect(formula).toBeEnabled();
    await expect(toolbar.getByRole("button").last()).toHaveAttribute("aria-label", "标记为公式");
    await formula.hover();
    const hint = page.getByRole("tooltip");
    await expect(hint.locator(".control-hint-content")).toHaveText("标记为公式");
    await expect(hint.locator("kbd")).toHaveText("⌘⇧E");
    await expect(formula).toHaveCSS("height", "28px");
    await page.screenshot({ path: test.info().outputPath("formula-toolbar.png") });
    await formula.click();
    await expect(editor.locator(".inline-equation-preview")).toHaveCount(1);
    await expect(editor.locator(".inline-equation-preview .katex")).toBeVisible();
    await expect(page.getByRole("dialog", { name: "编辑行内公式", exact: true })).toHaveCount(0);
    await expect(editor).toBeFocused();
    await page.keyboard.press("ControlOrMeta+z");
    await expect(editor.locator(".inline-equation-preview")).toHaveCount(0);
    await expect(editor).toHaveText("E=mc^2");
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.press("ControlOrMeta+Shift+E");
    await expect(editor.locator(".inline-equation-preview")).toHaveCount(1);
    await page.keyboard.type(" after");
    await expect(editor).toContainText("after");
    await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
    await page.reload(); await page.getByRole("button", { name: "笔记", exact: true }).click();
    await page.getByRole("tab", { name: "页面", exact: true }).click();
    await page.getByRole("tree").getByRole("button", { name: "Formula selection", exact: true }).click();
    await expect(editor.locator(".inline-equation-preview")).toHaveCount(1);
    await expect(editor).toContainText("after");
  });
}
