import { expect, test } from "@playwright/test";

for (const url of ["/", "/?preview=app"]) {
  test(`Tab confirms a note suggestion while Enter stays a newline on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
    const editor = page.locator(".notes-content .bn-editor");
    const menu = page.getByRole("listbox");
    await editor.click(); await page.keyboard.type("/");
    await expect(menu.getByRole("option").first()).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("ArrowDown");
    await expect(menu.getByRole("option").nth(1)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("Tab");
    await expect(menu).toHaveCount(0);
    await expect(editor.getByRole("heading", { level: 2 })).toBeVisible();
    await page.keyboard.type("Chosen by Tab");
    await page.keyboard.press("Enter");
    await page.keyboard.type("/"); await expect(menu).toBeVisible();
    await page.keyboard.press("Enter"); await expect(menu).toHaveCount(0);
    const blocks = editor.locator(".bn-block-content");
    await expect(blocks).toHaveCount(3);
    await expect(blocks.nth(1)).toHaveAttribute("data-content-type", "paragraph");
    await expect(blocks.nth(1)).toHaveText("/");
    await page.keyboard.type("Ordinary paragraph");
    await expect(blocks.last()).toHaveText("Ordinary paragraph");
    await page.keyboard.press("Enter");
    await page.keyboard.type("/inline equation");
    await expect(menu.getByText("行内公式", { exact: true })).toBeVisible();
    await editor.dispatchEvent("keydown", { key: "Enter", keyCode: 229, isComposing: true });
    await expect(menu).toBeVisible();
    await page.keyboard.press("Tab");
    await expect(menu).toHaveCount(0);
    const input = page.getByRole("textbox", { name: "行内公式 LaTeX 源码", exact: true });
    await expect(input).toBeFocused();
    await input.fill("x^2"); await input.press("Enter");
    await expect(input).toHaveCount(0);
    await expect(page.getByRole("button", { name: "行内公式 x^2，点击编辑", exact: true })).toBeVisible();
    await page.keyboard.type("/"); await expect(menu).toBeVisible();
    await page.keyboard.press("Escape"); await expect(menu).toHaveCount(0);
  });
}
