import { expect, test, type Locator } from "@playwright/test";

async function selectBodyText(editor: Locator): Promise<void> {
  await editor.focus();
  await editor.locator('.bn-block-content:not([data-content-type="documentTitle"]) .bn-inline-content').evaluateAll(nodes => {
    const first = nodes[0], last = nodes.at(-1);
    if (!first || !last) throw new Error("Missing note body");
    const range = document.createRange();
    range.setStart(first, 0);
    range.setEnd(last, last.childNodes.length);
    const selection = document.getSelection();
    if (!selection) throw new Error("Missing note selection");
    selection.removeAllRanges();
    selection.addRange(range);
  });
}

for (const url of ["/", "/?preview=app"]) {
  test(`color grid applies, remembers and clears formatting without losing selection on ${url}`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 808 });
    await page.goto(url); await page.getByRole("button", { name: "笔记", exact: true }).click();
    const editor = page.locator(".notes-content .bn-editor");
    const previous = await editor.count() ? await editor.elementHandle() : null;
    await page.locator(".notes-explorer").getByRole("button", { name: "新建独立笔记", exact: true }).click();
    if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
    await editor.click(); await page.keyboard.type("Color grid check"); await selectBodyText(editor);
    const toolbar = page.locator(".bn-formatting-toolbar");
    const open = async () => { await toolbar.getByRole("button", { name: "颜色", exact: true }).click();
      await expect(page.getByRole("menu", { name: "文本与背景颜色", exact: true })).toBeVisible(); };
    const menu = page.getByRole("menu", { name: "文本与背景颜色", exact: true });
    await expect(toolbar.locator("button, [role=combobox]").nth(0)).toHaveAttribute("aria-label", "问AI");
    await expect(toolbar.locator("button, [role=combobox]").nth(1)).toHaveAttribute("role", "combobox");
    await expect(toolbar.locator("button, [role=combobox]").nth(2)).toHaveAttribute("aria-label", "颜色");
    await open();
    await expect(menu).toHaveCSS("width", "186px");
    await expect(menu.getByRole("menuitemradio").first()).toHaveCSS("width", "28px");
    await expect(menu.getByRole("menuitemradio").first()).toHaveCSS("height", "28px");
    await expect(menu.getByRole("menuitemradio")).toHaveCount(20);
    await expect(menu.getByRole("region", { name: "最近使用" })).toHaveCount(0);
    await expect(menu.locator(".block-color-grid").first()).toHaveCSS("display", "grid");
    await expect(menu.getByRole("menuitemradio", { name: "文本颜色 · 自动", exact: true })).toHaveAttribute("aria-checked", "true");
    await menu.getByRole("menuitemradio", { name: "文本颜色 · 红色", exact: true }).click();
    await expect(menu).toBeHidden(); await expect(editor).toBeFocused();
    await expect(editor.locator('[data-style-type="textColor"][data-value="red"]')).toContainText("Color grid check");
    await open();
    await expect(menu.getByRole("menuitemradio", { name: "最近使用 · 文本颜色 · 红色", exact: true })).toBeVisible();
    await menu.getByRole("menuitemradio", { name: "背景颜色 · 黄色", exact: true }).click();
    await expect(editor.locator('[data-style-type="backgroundColor"][data-value="yellow"]')).toContainText("Color grid check");
    await toolbar.getByRole("button", { name: "加粗", exact: true }).click();
    await toolbar.getByRole("button", { name: "下划线", exact: true }).click();
    await open();
    await expect(menu.getByRole("menuitemradio", { name: /^最近使用/ })).toHaveCount(2);
    for (const name of ["灰色", "棕色", "橙色", "绿色", "蓝色", "紫色", "红色"]) {
      await menu.getByRole("menuitemradio", { name: `文本颜色 · ${name}`, exact: true }).click();
      await open();
    }
    await expect(menu.getByRole("menuitemradio", { name: /^最近使用/ })).toHaveCount(5);
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/note-color-grid.png" });
    await page.keyboard.press("Escape");
    const clear = toolbar.getByRole("button", { name: "清除样式", exact: true });
    await clear.hover(); await expect(page.getByRole("tooltip")).toContainText("清除样式");
    await clear.click();
    await expect(editor.locator('strong, u, [data-style-type="textColor"][data-value="red"], [data-style-type="backgroundColor"][data-value="yellow"]')).toHaveCount(0);
    await expect(editor).toContainText("Color grid check"); await expect(editor).toBeFocused();
    await page.keyboard.press("ControlOrMeta+z");
    await expect(editor.locator("strong")).toContainText("Color grid check");
    await expect(editor.locator("u")).toContainText("Color grid check");
    await expect(editor.locator('[data-style-type="textColor"][data-value="red"]')).toContainText("Color grid check");
    await expect(editor.locator('[data-style-type="backgroundColor"][data-value="yellow"]')).toContainText("Color grid check");
    await page.setViewportSize({ width: 375, height: 667 });
    await open();
    await expect.poll(() => menu.evaluate(el => {
      const rect = el.getBoundingClientRect(); return rect.left >= 0 && rect.right <= innerWidth && rect.top >= 0 && rect.bottom <= innerHeight;
    })).toBe(true);
    await page.keyboard.press("Escape");
    await editor.focus();
    await editor.locator(".bn-inline-content").first().evaluate(element => {
      const range = document.createRange(); range.selectNodeContents(element); range.collapse(false);
      const selection = document.getSelection()!; selection.removeAllRanges(); selection.addRange(range);
    });
    await page.keyboard.press("Enter");
    await expect(editor).toContainText("Color grid check");
    await page.keyboard.type("Plain text");
    await editor.locator(".bn-inline-content").last().evaluate(element => {
      const range = document.createRange(); range.selectNodeContents(element);
      const selection = document.getSelection()!; selection.removeAllRanges(); selection.addRange(range);
    });
    await toolbar.getByRole("button", { name: "清除样式", exact: true }).click();
    await expect(editor.locator("strong")).toContainText("Color grid check");
    await expect(editor.locator(".bn-block-content").last().locator("strong, u, [data-style-type]")).toHaveCount(0);
    await selectBodyText(editor);
    await toolbar.getByRole("button", { name: "清除样式", exact: true }).click();
    await expect(editor.locator('strong, u, [data-style-type="textColor"][data-value="red"], [data-style-type="backgroundColor"][data-value="yellow"]')).toHaveCount(0);
    await expect(editor).toContainText("Plain text");
  });
}
