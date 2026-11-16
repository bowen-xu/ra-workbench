import { expect, test } from "@playwright/test";

for (const url of ["/", "/?preview=app"]) {
  const mode = url === "/" ? "browser" : "preview";
  test(`notes fills the available height in ${mode} mode and scrolls inside its panes`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 808 });
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    const empty = page.locator(".note-empty-state");
    await expect(empty.getByRole("heading", { name: "建立你的研究笔记" })).toBeVisible();
    for (const size of [{ width: 1276, height: 808 }, { width: 900, height: 600 }, { width: 650, height: 900 }]) {
      await page.setViewportSize(size);
      await expect.poll(() => page.locator(".notes-content").evaluate(element => {
        const content = element.getBoundingClientRect(), state = element.querySelector(".note-empty-state")!.getBoundingClientRect();
        return { topGap: Math.round(state.top - content.top), bottomGap: Math.round(content.bottom - state.bottom), viewportGap: Math.round(window.innerHeight - content.bottom) };
      })).toEqual({ topGap: 0, bottomGap: 0, viewportGap: 0 });
      expect(await page.locator(".main-area").evaluate(e => e.scrollHeight - e.clientHeight)).toBeLessThanOrEqual(1);
      expect(await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)).toBeLessThanOrEqual(1);
    }
    await page.setViewportSize({ width: 1276, height: 808 });
    if (mode === "preview") await page.screenshot({ path: ".tmp/notes-empty-preview.png" });
    await empty.getByRole("button", { name: "新建页面" }).click();
    await expect(page.getByRole("textbox", { name: "笔记标题" })).toBeVisible();
    await expect(page.getByRole("navigation", { name: "页面路径" })).toHaveCount(0);
    const status = page.locator(".note-page-toolbar .note-save-state");
    await expect(status).toHaveAttribute("aria-label", "已保存");
    await expect(status).toHaveText("");
    await page.getByRole("button", { name: "收藏页面", exact: true }).focus();
    await page.keyboard.press("Shift+Tab");
    await expect(page.getByRole("tooltip", { name: "已保存" })).toBeVisible();
    await page.getByRole("textbox", { name: "笔记标题" }).click();
    await page.getByRole("button", { name: "更多页面操作" }).click();
    await expect(page.getByRole("menuitem", { name: "上移页面" })).toHaveCount(0);
    await expect(page.getByRole("menuitem", { name: "下移页面" })).toHaveCount(0);
    await expect(page.getByRole("menuitem")).toHaveText(["出链与反链", "版本历史", "创建副本", "归档", "移到回收站"]);
    if (mode === "preview") await page.screenshot({ path: ".tmp/notes-menu-preview.png" });
    await page.getByRole("menuitem", { name: "版本历史" }).click();
    await expect(page.getByRole("complementary", { name: "版本历史" })).toBeVisible();
    await page.getByRole("button", { name: "关闭版本历史" }).click();
    await page.setViewportSize({ width: 650, height: 900 });
    await expect(status).toBeVisible();
    await page.getByRole("button", { name: "收藏页面", exact: true }).focus();
    await page.keyboard.press("Shift+Tab");
    await expect(page.getByRole("tooltip", { name: "已保存" })).toBeVisible();
    await page.emulateMedia({ reducedMotion: "reduce" });
    await expect(status).toBeVisible();
    await page.locator(".bn-editor").click();
    for (let index = 0; index < 50; index++) {
      await page.keyboard.insertText(`笔记正文第 ${index + 1} 行`); await page.keyboard.press("Enter");
    }
    await expect(status).toHaveAttribute("aria-label", "已保存");
    await expect.poll(() => page.locator(".notes-content").evaluate(e => e.scrollHeight > e.clientHeight)).toBe(true);
    expect(await page.locator(".main-area").evaluate(e => e.scrollHeight - e.clientHeight)).toBeLessThanOrEqual(1);
    expect(await page.evaluate(() => document.documentElement.scrollHeight - window.innerHeight)).toBeLessThanOrEqual(1);
  });
}
