import { expect, test, type Page } from "@playwright/test";

async function openMenu(page: Page) {
  const editor = page.locator(".notes-content .bn-editor");
  const previous = await editor.count() ? await editor.elementHandle() : null;
  await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
  if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
  await expect(editor).toBeVisible();
  await editor.click();
  await page.keyboard.type("/");
  const menu = page.getByRole("listbox");
  await expect(menu.getByRole("option").first()).toHaveAttribute("aria-selected", "true");
  return menu;
}

for (const url of ["/", "/?preview=app"]) {
  test(`pointer and keyboard share one active suggestion and execute it on ${url}`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 808 });
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    let menu = await openMenu(page);
    const first = menu.getByRole("option", { name: /一级标题/ });
    const third = menu.getByRole("option", { name: /三级标题/ });
    await third.hover();
    await expect(third).toHaveAttribute("aria-selected", "true");
    await expect(first).not.toHaveAttribute("aria-selected", "true");
    await expect(menu.locator('[role="option"][aria-selected="true"]')).toHaveCount(1);
    await expect(page.locator(".bn-editor")).toHaveAttribute("aria-activedescendant", await third.getAttribute("id") ?? "");
    await page.keyboard.press("ArrowDown");
    const quote = menu.getByRole("option", { name: /^引用/ });
    await expect(quote).toHaveAttribute("aria-selected", "true");
    await expect(third).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
    await expect(menu.locator('[role="option"][aria-selected="true"]')).toHaveCount(1);
    await page.keyboard.press("ArrowUp");
    await expect(third).toHaveAttribute("aria-selected", "true");
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/note-suggestion-single-active.png" });
    await page.keyboard.press("Tab");
    await expect(menu).toHaveCount(0);
    await expect(page.locator('.bn-block-content[data-content-type="heading"][data-level="3"]')).toBeVisible();
    await page.keyboard.type("Chosen third heading");
    await expect(page.locator(".bn-editor").getByRole("heading", { level: 3 })).toHaveText("Chosen third heading");

    menu = await openMenu(page);
    await menu.getByRole("option", { name: /二级标题/ }).click();
    await expect(menu).toHaveCount(0);
    await page.keyboard.type("Chosen second heading");
    await expect(page.locator(".bn-editor").getByRole("heading", { level: 2 })).toHaveText("Chosen second heading");

    menu = await openMenu(page);
    await menu.getByRole("option", { name: /三级标题/ }).hover();
    await page.keyboard.press("PageUp");
    await expect(menu.getByRole("option").first()).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("PageDown");
    await expect(menu.getByRole("option").last()).toHaveAttribute("aria-selected", "true");
    await expect(menu.locator('[role="option"][aria-selected="true"]')).toHaveCount(1);
    await page.keyboard.type("引用");
    await expect(menu.getByRole("option", { name: /^引用/ })).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("Tab");
    await expect(menu).toHaveCount(0);
    await expect(page.locator('.bn-block-content[data-content-type="quote"]')).toHaveCount(1);
  });
}
