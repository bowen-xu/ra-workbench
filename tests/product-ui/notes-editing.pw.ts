import { expect, test } from "@playwright/test";

async function createFreshPage(page: import("@playwright/test").Page) {
  const editors = page.locator(".notes-content .bn-editor");
  const previous = await editors.count() ? await editors.elementHandle() : null;
  await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
  if (previous) await expect.poll(() => previous.evaluate(e => e.isConnected)).toBe(false);
  await expect(page.locator(".notes-content .bn-editor")).toBeVisible();
  await page.getByRole("textbox", { name: "笔记标题" }).fill(`输入转换测试 ${Date.now()}`);
}

test("reading notes edit in the right pane with correct labels and durable page switching", async ({ page }) => {
  await page.goto("/?preview=app");
  await page.getByRole("row", { name: /Synthetic Research Preview/ }).dblclick();
  await page.getByRole("tab", { name: "高亮与笔记", exact: true }).click();
  await page.getByRole("button", { name: "写文献笔记", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "文献笔记编辑面板", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "关闭阅读笔记", exact: true }).click();
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  const list = page.locator(".notes-explorer .reading-note-list");
  await expect(list.getByRole("button", { name: /^文献笔记 文献笔记/ })).toBeVisible();
  await expect(list.getByRole("button", { name: /^标注 这是一条合成批注/ })).toBeVisible();
  await list.getByRole("button", { name: /^标注 这是一条合成批注/ }).click();
  await expect(page.getByRole("dialog", { name: /笔记编辑面板/ })).toHaveCount(0);
  const editor = page.locator(".notes-content .bn-editor");
  await expect(editor).toContainText("这是一条合成批注");
  await editor.click(); await editor.press("ControlOrMeta+End"); await editor.press("Enter"); await editor.pressSequentially("保存在右侧的阅读批注");
  await expect(page.locator(".notes-content .note-save-state")).toHaveAttribute("aria-label", "已保存");
  await page.getByRole("tab", { name: "阅读笔记", exact: true }).click();
  await expect(editor).toContainText("保存在右侧的阅读批注");
  await page.getByRole("tab", { name: "全部笔记", exact: true }).click();
  await list.getByRole("button", { name: /^文献笔记 文献笔记/ }).click();
  await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("文献笔记");
  await list.getByRole("button", { name: /^标注 .*保存在右侧的阅读批注/ }).click();
  await expect(editor).toContainText("保存在右侧的阅读批注");
  await expect(page.getByRole("dialog", { name: /笔记编辑面板/ })).toHaveCount(0);
  await page.screenshot({ path: ".tmp/notes-inline-reading.png" });
});

test("notes menus use shared typography and actual typed Markdown input shortcuts", async ({ page }) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  for (const [text, type, level] of [["# ", "heading", "1"], ["## ", "heading", "2"], ["### ", "heading", "3"], ['" ', "quote", ""], ["[] ", "checkListItem", ""], ["- ", "bulletListItem", ""], ["1. ", "numberedListItem", ""]]) {
    await createFreshPage(page);
    const editor = page.locator(".notes-content .bn-editor");
    await editor.click(); await editor.pressSequentially(text!);
    const block = editor.locator(".bn-block-content").first();
    await expect(block).toHaveAttribute("data-content-type", type!);
    if (level) await expect(block.getByRole("heading", { level: Number(level) })).toBeVisible();
    await editor.press("ControlOrMeta+z");
    await expect(block).toHaveAttribute("data-content-type", "paragraph");
  }
  await createFreshPage(page);
  await page.locator(".bn-editor").click(); await page.keyboard.type("/");
  const menu = page.getByRole("listbox");
  await expect(menu.locator(".bn-ak-suggestion-menu-item-title").first()).toHaveCSS("font-size", "14px");
  await expect(menu.locator(".bn-ak-suggestion-menu-item-subtitle").first()).toBeHidden();
  await expect(menu.locator(".bn-ak-group-label").first()).toHaveCSS("font-size", "11px");
  await expect(menu.getByText("#", { exact: true })).toBeVisible();
  await page.screenshot({ path: ".tmp/notes-slash-menu.png" });
});

for (const url of ["/", "/?preview=app"]) {
  test(`notes keep minimum side whitespace and aligned document sections on ${url}`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 808 });
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await createFreshPage(page);
    await page.locator(".bn-editor").click();
    await page.keyboard.type("Whitespace layout check");
    for (const width of [1276, 1800, 960, 375]) {
      await page.setViewportSize({ width, height: 808 });
      await expect.poll(() => page.locator(".notes-content").evaluate(pane => {
        const bounds = pane.getBoundingClientRect();
        const header = pane.querySelector(".note-document-header")!.getBoundingClientRect();
        const body = pane.querySelector(".bn-block-content")!.getBoundingClientRect();
        const left = body.left - bounds.left - pane.clientLeft;
        const right = bounds.left + pane.clientLeft + pane.clientWidth - body.right;
        if (innerWidth <= 700) return left >= 24 && left <= 30 && right >= 24 && right <= 30
          && pane.scrollWidth <= pane.clientWidth + 1;
        return left >= 89.5 && right >= 89.5 && body.width <= 761
          && Math.abs(header.left - body.left) < 1 && Math.abs(header.right - body.right) < 1
          && pane.scrollWidth <= pane.clientWidth + 1;
      })).toBe(true);
      if (width === 1276 && url.includes("preview")) await page.screenshot({ path: ".tmp/notes-side-whitespace-90.png" });
    }
  });

  test(`the note slash menu is compact and scrolls within available space on ${url}`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 808 });
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await createFreshPage(page);
    await page.getByRole("textbox", { name: "笔记标题" }).fill("文献笔记");
    await page.locator(".bn-editor").click(); await page.keyboard.type("/");
    const menu = page.locator(".note-slash-menu-popover").getByRole("listbox");
    await expect(menu).toHaveCSS("width", "240px");
    await expect(menu).toHaveCSS("height", "300px");
    expect(await menu.evaluate(e => e.scrollHeight > e.clientHeight)).toBe(true);
    expect(await menu.evaluate(e => e.scrollWidth - e.clientWidth)).toBeLessThanOrEqual(1);
    if (url.includes("preview")) {
      await expect(page.locator(".note-slash-menu-popover")).toHaveCSS("opacity", "1");
      await page.screenshot({ path: ".tmp/notes-compact-slash-menu.png" });
    }
    for (const size of [{ width: 375, height: 667 }, { width: 220, height: 400 }]) {
      await page.setViewportSize(size);
      await expect.poll(() => menu.evaluate(e => {
        const rect = e.getBoundingClientRect();
        return rect.width <= Math.min(240, innerWidth - 20) + 1 && rect.height <= 300
          && rect.left >= -1 && rect.right <= innerWidth + 1 && rect.top >= -1 && rect.bottom <= innerHeight + 1;
      })).toBe(true);
      expect(await menu.evaluate(e => e.scrollWidth - e.clientWidth)).toBeLessThanOrEqual(1);
    }
    await page.setViewportSize({ width: 1276, height: 808 });
    const items = menu.getByRole("option");
    for (let index = 1; index < await items.count(); index++) await page.keyboard.press("ArrowDown");
    await expect(items.last()).toHaveAttribute("aria-selected", "true");
    await expect.poll(() => menu.evaluate(e => e.scrollTop)).toBeGreaterThan(0);
    const last = await items.last().boundingBox(), box = await menu.boundingBox();
    expect(last!.y + last!.height).toBeLessThanOrEqual(box!.y + box!.height);
    await page.keyboard.press("Escape"); await expect(menu).toHaveCount(0);
  });
}

test("export asks for a directory and reports success through the shared notification tool", async ({ page }) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await page.getByRole("button", { name: "导出页面树", exact: true }).click();
  const dialog = page.getByRole("dialog", { name: "导出笔记", exact: true });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("button", { name: "导出", exact: true })).toBeDisabled();
  await dialog.getByRole("button", { name: "选择目录", exact: true }).click();
  await expect(dialog.getByRole("textbox", { name: "笔记导出目录" })).toHaveValue("/Users/preview/Documents");
  await dialog.getByRole("textbox", { name: "笔记导出目录" }).fill("/Users/preview/Documents/研究笔记");
  await page.screenshot({ path: ".tmp/notes-export-directory.png" });
  await dialog.getByRole("button", { name: "导出", exact: true }).click();
  await expect(dialog).toHaveCount(0);
  await expect(page.locator("[data-sonner-toast]").filter({ hasText: "已导出" })).toBeVisible();
  await expect(page.locator(".notes-content .notice-banner, .notes-content .error-banner")).toHaveCount(0);
});

test("the full-width toolbar stays at the top with a fading lower edge and button hints", async ({ page }) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await createFreshPage(page);
  const toolbar = page.locator(".note-page-topline"), pane = page.locator(".notes-content");
  for (const name of ["收藏页面", "页面属性", "更多页面操作"]) {
    await toolbar.getByRole("button", { name, exact: true }).hover();
    await expect(page.getByRole("tooltip", { name, exact: true })).toBeVisible();
  }
  await page.locator(".bn-editor").click();
  for (let index = 0; index < 60; index++) {
    await page.keyboard.insertText("较长的笔记正文"); await page.keyboard.press("Enter");
  }
  await expect.poll(() => pane.evaluate(e => e.scrollHeight - e.clientHeight)).toBeGreaterThan(0);
  await pane.hover(); await page.mouse.wheel(0, 1400);
  await expect.poll(() => pane.evaluate(e => e.scrollTop)).toBeGreaterThan(0);
  const top = await pane.boundingBox(), fixed = await toolbar.boundingBox();
  expect(Math.abs(top!.y - fixed!.y)).toBeLessThan(1);
  expect(Math.abs(top!.width - fixed!.width)).toBeLessThan(1);
  await expect(toolbar).toHaveCSS("padding-left", "5px"); await expect(toolbar).toHaveCSS("padding-right", "5px");
  expect(await toolbar.evaluate(e => getComputedStyle(e, "::after").backgroundImage)).toContain("linear-gradient");
  await page.screenshot({ path: ".tmp/notes-sticky-toolbar.png" });
});
