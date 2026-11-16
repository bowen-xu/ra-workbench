import { expect, test } from "@playwright/test";

for (const url of ["/", "/?preview=app"]) {
  test(`note hyperlinks use dark text and the reference underline on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    const editor = page.locator(".notes-content .bn-editor");
    const previous = await editor.count() ? await editor.elementHandle() : null;
    await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
    if (previous) await expect.poll(() => previous.evaluate(element => element.isConnected)).toBe(false);
    await page.getByRole("textbox", { name: "笔记标题" }).fill("Hyperlink style");
    await editor.click(); await page.keyboard.type("Hyperlink label");
    await page.keyboard.press("ControlOrMeta+a");
    await page.locator('.bn-formatting-toolbar [data-test="createLink"]').click();
    await page.locator('input[name="url"]').fill("https://example.org/research");
    await page.locator('input[name="url"]').press("Enter");
    await page.keyboard.press("ArrowRight");
    const link = editor.getByRole("link", { name: "Hyperlink label", exact: true });
    await expect(link).toHaveCSS("color", "rgb(84, 80, 73)");
    await expect(link).toHaveCSS("text-decoration-line", "underline");
    await expect(link).toHaveCSS("text-decoration-style", "dashed");
    await expect(link).toHaveCSS("text-decoration-thickness", "1px");
    await expect(link).toHaveCSS("text-underline-offset", "3px");
    await expect(link).toHaveAttribute("href", "https://example.org/research");
    await link.hover();
    await expect(link).toHaveCSS("color", "rgb(84, 80, 73)");
    await expect(link).toHaveCSS("text-decoration-style", "dashed");
    await expect(page.locator(".bn-link-toolbar").getByRole("button", { name: "编辑链接", exact: true })).toBeVisible();
    await page.mouse.move(10, 60);
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/note-hyperlink-style.png" });
    await editor.focus(); await page.keyboard.press("ControlOrMeta+a");
    await page.locator(".bn-formatting-toolbar").getByRole("button", { name: "下划线", exact: true }).click();
    await page.locator(".bn-formatting-toolbar").getByRole("button", { name: "颜色", exact: true }).click();
    await page.getByRole("menuitemradio", { name: "文本颜色 · 橙色", exact: true }).click();
    const color = await editor.locator('[data-style-type="textColor"][data-value="orange"]').first().evaluate(element => getComputedStyle(element).color);
    await expect(link).toHaveCSS("color", color);
    await expect(editor.locator("u")).toHaveCSS("text-decoration-color", color);
    expect(await link.evaluate(element => {
      const probe = document.createElement("span");
      probe.style.color = getComputedStyle(element).color;
      probe.style.textDecorationColor = "color-mix(in srgb, currentColor 30%, transparent)";
      document.body.append(probe);
      const expected = getComputedStyle(probe).textDecorationColor; probe.remove();
      return getComputedStyle(element).textDecorationColor === expected;
    })).toBe(true);
    await page.locator(".bn-formatting-toolbar").getByRole("button", { name: "清除样式", exact: true }).click();
    await expect(link).toHaveCSS("color", "rgb(84, 80, 73)");
    await expect(link).toHaveCSS("text-decoration-style", "dashed");
  });
}
