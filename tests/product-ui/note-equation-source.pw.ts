import { expectEquationSource } from "../ui/equation-source";
import { expect, test } from "@playwright/test";

for (const url of ["/", "/?preview=app"]) {
  test(`formula source supports newline and literal Tab without resizing or implicit save on ${url}`, async ({ page }) => {
    await page.goto(url); await page.getByRole("button", { name: "笔记", exact: true }).click();
    const editor = page.locator(".notes-content .bn-editor");
    const previous = await editor.count() ? await editor.elementHandle() : null;
    await page.locator(".notes-explorer").getByRole("button", { name: "新建独立笔记", exact: true }).click();
    if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
    await editor.click(); await page.keyboard.type("$"); await page.keyboard.press("Tab");
    const inline = page.getByRole("textbox", { name: "行内公式 LaTeX 源码", exact: true });
    await inline.fill("E=mc^2"); await inline.press("End"); await inline.press("Shift+Enter");
    await expect(inline).toBeFocused(); await expectEquationSource(inline, "E=mc^2\n");
    await inline.press("Tab"); await page.keyboard.type("a+b");
    await expectEquationSource(inline, "E=mc^2\n\ta+b");
    await expect(inline).toHaveCSS("resize", "none");
    await expect(page.locator(".inline-equation-preview")).toHaveAttribute("aria-label", "空行内公式，点击编辑");
    await inline.press("Enter"); await expect(inline).toHaveCount(0);
    await page.locator(".inline-equation-preview").click();
    await expectEquationSource(inline, "E=mc^2\n\ta+b");
    await inline.press("ControlOrMeta+a"); await inline.press("Tab");
    await expectEquationSource(inline, "\t"); await inline.press("Escape");
    await page.locator(".inline-equation-preview").click();
    await expectEquationSource(inline, "E=mc^2\n\ta+b"); await inline.press("Enter");
    await page.keyboard.press("Enter"); await page.keyboard.type("$$"); await page.keyboard.press("Tab");
    const block = page.locator(".note-formula-disclosure"); await block.click();
    const source = page.getByRole("textbox", { name: "公式 LaTeX 源码", exact: true });
    await expect(source).toHaveCSS("resize", "none");
    await source.fill("x"); await source.press("End"); await source.press("Tab");
    await expectEquationSource(source, "x\t"); await expect(source).toBeFocused();
    await source.press("Shift+Enter"); await page.keyboard.type("y");
    await expectEquationSource(source, "x\t\ny");
    await source.press("Enter"); await expect(source).toHaveCount(0);
    await block.click(); await expectEquationSource(source, "x\t\ny");
  });
}
