import { expect, test } from "@playwright/test";

for (const url of ["/", "/?preview=app"]) {
  test(`formatting toolbar uses shared hints without changing selection or controls on ${url}`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 808 });
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    const editor = page.locator(".notes-content .bn-editor");
    const previous = await editor.count() ? await editor.elementHandle() : null;
    await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
    if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
    await editor.click(); await page.keyboard.type("Toolbar hint check"); await page.keyboard.press("ControlOrMeta+a");
    const toolbar = page.locator(".bn-formatting-toolbar");
    const underline = toolbar.getByRole("button", { name: "下划线", exact: true });
    await expect(underline).toBeVisible();
    await underline.hover();
    const hint = page.getByRole("tooltip");
    await expect(hint).toHaveCount(1);
    await expect(hint).toHaveClass(/control-hint/);
    await expect(hint.locator(".control-hint-content")).toHaveText("下划线");
    await expect(hint.locator("kbd")).toContainText("U");
    await expect(hint).toHaveCSS("font-size", "11px");
    await expect(hint).toHaveCSS("background-color", "rgb(31, 42, 36)");
    await expect(hint).toHaveCSS("pointer-events", "none");
    await expect(page.locator(".bn-ak-tooltip")).toHaveCount(0);
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/note-toolbar-control-hint.png" });
    const hintBox = await hint.boundingBox();
    await page.mouse.move(hintBox!.x + hintBox!.width / 2, hintBox!.y + hintBox!.height / 2);
    await expect(hint).toHaveCount(0);
    await underline.click();
    await expect(editor.locator("u")).toHaveText("Toolbar hint check");
    await expect(underline).toHaveAttribute("aria-pressed", "true");
    await expect(editor).toBeFocused();
    const bold = toolbar.getByRole("button", { name: "加粗", exact: true });
    await bold.click(); await expect(editor.locator("strong")).toHaveText("Toolbar hint check");
    // Color picker uses a toolbar button as a menu trigger: forwarded refs and
    // Ariakit-injected props must continue to open its normal command menu.
    await toolbar.getByRole("button", { name: "颜色", exact: true }).click();
    await expect(page.getByRole("menu")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("menu")).toHaveCount(0);
    const formattingSurface = await toolbar.evaluate(element => {
      const style = getComputedStyle(element);
      return { shadow: style.boxShadow, radius: style.borderRadius, padding: style.padding, gap: style.gap };
    });
    await toolbar.locator('[data-test="createLink"]').click();
    await page.locator('input[name="url"]').fill("https://example.org/method");
    await page.locator('input[name="url"]').press("Enter");
    await page.keyboard.press("ArrowRight");
    const link = editor.getByRole("link", { name: "Toolbar hint check", exact: true });
    await link.hover();
    const links = page.locator(".bn-link-toolbar");
    const edit = links.getByRole("button", { name: "编辑链接", exact: true });
    await expect(edit).toBeVisible();
    expect(await links.evaluate(element => {
      const style = getComputedStyle(element);
      return { shadow: style.boxShadow, radius: style.borderRadius, padding: style.padding, gap: style.gap };
    })).toEqual(formattingSurface);
    for (const button of await links.getByRole("button").all()) {
      await expect(button).toHaveCSS("height", "28px");
      await expect(button).toHaveCSS("font-size", "13px");
      await expect(button).toHaveCSS("box-shadow", "none");
    }
    await expect(links.locator(".bn-ak-button svg").first()).toHaveCSS("width", "14px");
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/note-link-toolbar.png" });
    await edit.click();
    await expect(page.locator('input[name="url"]')).toHaveValue("https://example.org/method");
    await page.locator('input[name="url"]').fill("https://example.org/revised");
    await page.locator('input[name="url"]').press("Enter");
    await expect(link).toHaveAttribute("href", "https://example.org/revised");
  });
}
