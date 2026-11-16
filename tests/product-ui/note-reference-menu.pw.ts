import { expect, test, type Page } from "@playwright/test";

async function newPage(page: Page, title: string) {
  const editor = page.locator(".notes-content .bn-editor");
  const previous = await editor.count() ? await editor.elementHandle() : null;
  await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
  if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
  await expect(editor).toBeVisible();
  await page.getByRole("textbox", { name: "笔记标题" }).fill(title);
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  await editor.click();
  return editor;
}

for (const url of ["/", "/?preview=app"]) {
  test(`slash holds commands and at-sign inserts references inline on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await newPage(page, "Reference target");
    const editor = await newPage(page, "Reference current");
    await page.keyboard.type("/");
    const menu = page.getByRole("listbox");
    await expect(menu).toBeVisible();
    await expect(menu.getByRole("option", { name: /^引用/ })).toBeVisible();
    await expect(menu.getByRole("option", { name: /^页面 ·|^阅读笔记 ·/ })).toHaveCount(0);
    await expect(menu.getByText("页面引用", { exact: true })).toHaveCount(0);
    await expect(menu.getByText("阅读笔记引用", { exact: true })).toHaveCount(0);
    await page.keyboard.press("Escape"); await page.keyboard.press("Backspace");
    await page.keyboard.type("  @Reference target");
    await menu.getByRole("option", { name: /^页面 · Reference target/ }).click();
    await expect(menu).toHaveCount(0);
    await page.keyboard.type(" After");
    await expect(editor.locator(".bn-block-content")).toHaveCount(1);
    const mention = editor.locator(".research-mention");
    await expect(mention).toHaveText("Reference target");
    await expect(editor).toContainText("After");
    const target = await mention.getAttribute("data-target-id");
    await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
    await page.keyboard.press("Enter"); await page.keyboard.type("@");
    await expect(menu.getByRole("option", { name: /^页面 · Reference current/ })).toHaveCount(0);
    await expect(menu.getByRole("option", { name: /^页面 · Reference target/ })).toBeVisible();
    if (url.includes("preview")) {
      const reading = menu.getByRole("option", { name: /^阅读笔记 ·/ }).first();
      await expect(reading).toBeVisible();
      const label = await reading.locator(".bn-ak-suggestion-menu-item-title").textContent();
      await reading.click();
      await expect(editor.locator(".research-mention")).toHaveCount(2);
      await expect(editor.locator(".research-mention").last()).toHaveText(`${label!.replace("阅读笔记 · ", "")}`);
      await expect(editor.locator('[data-content-type="readingNoteReference"]')).toHaveCount(0);
    } else await page.keyboard.press("Escape");
    await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
    await page.reload();
    if (!await page.locator(".notes-content").count()) await page.getByRole("button", { name: "笔记", exact: true }).click();
    await page.getByRole("tab", { name: "全部笔记", exact: true }).click();
    await page.locator(".reading-note-list").getByRole("button", { name: /^独立笔记 Reference current/ }).click();
    await expect(editor.locator(`.research-mention[data-target-id="${target}"]`)).toHaveText("Reference target");
    if (url.includes("preview")) await expect(editor.locator(".research-mention")).toHaveCount(2);
    await expect(editor).toContainText("After");
  });
}

for (const url of ["/", "/?preview=app"]) {
  test(`at-sign uses Tab confirmation, Enter newline and composition guards on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await newPage(page, "Tab target A");
    await newPage(page, "Tab target B");
    const editor = await newPage(page, "Tab reference current");
    await page.keyboard.type("  @Tab target");
    const menu = page.getByRole("listbox");
    const options = menu.getByRole("option");
    await expect(options).toHaveCount(2);
    await expect(options.first()).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowDown");
    await expect(options.nth(1)).toHaveAttribute("aria-selected", "true");
    const chosen = await options.nth(1).locator(".bn-ak-suggestion-menu-item-title").textContent();
    await editor.dispatchEvent("keydown", { key: "Tab", code: "Tab", keyCode: 229, isComposing: true });
    await expect(menu).toBeVisible(); await expect(editor.locator(".research-mention")).toHaveCount(0);
    await page.keyboard.press("Tab");
    await expect(menu).toHaveCount(0); await expect(editor).toBeFocused();
    await expect(editor.locator(".research-mention")).toHaveText(`${chosen!.replace("页面 · ", "")}`);
    await page.keyboard.type(" After");
    await expect(editor).toContainText("After");
    await page.keyboard.press("Enter"); await page.keyboard.type("@Tab target");
    await expect(options).toHaveCount(2);
    await editor.dispatchEvent("keydown", { key: "Enter", code: "Enter", keyCode: 229, isComposing: true });
    await expect(menu).toBeVisible();
    await page.keyboard.press("Enter"); await expect(menu).toHaveCount(0);
    const blocks = editor.locator(".bn-block-content");
    await expect(blocks).toHaveCount(3);
    await expect(blocks.nth(1)).toHaveText("@Tab target");
    await expect(editor.locator(".research-mention")).toHaveCount(1);
    await page.keyboard.type("Ordinary text"); await expect(blocks.last()).toHaveText("Ordinary text");
    await page.keyboard.press("Enter"); await page.keyboard.type("@NoSuchReference");
    await expect(options).toHaveCount(0);
    await page.keyboard.press("Tab");
    await expect(editor.locator(".research-mention")).toHaveCount(1);
    await expect(editor).toBeFocused();
    await page.keyboard.press("Escape"); await expect(menu).toHaveCount(0);
    await expect(blocks.last()).toHaveText("@NoSuchReference");
  });
}
