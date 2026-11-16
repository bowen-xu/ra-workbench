import { expectEquationSource } from "../ui/equation-source";
import { expect, test, type Page } from "@playwright/test";

async function fresh(page: Page, url: string) {
  await page.goto(url);
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  const previous = await page.locator(".notes-content .bn-editor").count()
    ? await page.locator(".notes-content .bn-editor").elementHandle() : null;
  await page.locator(".notes-explorer").getByRole("button", { name: "新建独立笔记", exact: true }).click();
  if (previous) await expect.poll(() => previous.evaluate(element => element.isConnected)).toBe(false);
  const editor = page.locator(".notes-content .bn-editor");
  await expect(editor).toBeVisible();
  await editor.click();
  return editor;
}

for (const url of ["/", "/?preview=app"]) {
  test(`inline arrows enter with all source selected and leave at exact boundaries on ${url}`, async ({ page }) => {
    const editor = await fresh(page, url);
    await page.keyboard.type("L $"); await page.keyboard.press("Tab");
    const input = page.getByRole("textbox", { name: "行内公式 LaTeX 源码", exact: true });
    await expect(input).toBeFocused(); await input.fill("E=mc^2"); await input.press("Enter");
    await expect(editor).toBeFocused(); await page.keyboard.type("R");
    await page.keyboard.press("ArrowLeft"); await page.keyboard.press("ArrowLeft");
    await expect(input).toHaveCount(0);
    await page.keyboard.press("ArrowLeft"); await expect(input).toBeFocused();
    expect(await input.evaluate(() => window.getSelection()?.toString())).toBe("E=mc^2");
    await input.press("ArrowLeft"); await expect(input).toBeVisible();
    await input.press("ArrowLeft"); await expect(input).toHaveCount(0); await expect(editor).toBeFocused();
    await page.keyboard.type("X");
    const formula = page.locator(".inline-equation-preview");
    expect(await formula.evaluate(el => el.closest(".react-renderer")!.previousSibling?.textContent)).toBe("L X");
    // Typing left of the atom leaves the caret immediately before it.
    await page.keyboard.press("ArrowRight"); await expect(input).toBeFocused();
    expect(await input.evaluate(() => window.getSelection()?.toString())).toBe("E=mc^2");
    await input.fill("discard me"); await input.press("End");
    await input.press("ArrowRight"); await expect(input).toHaveCount(0); await expect(editor).toBeFocused();
    await page.keyboard.type("Y");
    await expect(formula).toHaveAttribute("aria-label", "行内公式 E=mc^2，点击编辑");
    expect(await formula.evaluate(el => el.closest(".react-renderer")!.nextSibling?.textContent)).toBe("Y R");
    await page.keyboard.press("ArrowLeft"); await page.keyboard.press("ArrowLeft");
    await expect(input).toBeFocused();
    await input.press("ArrowRight"); await expect(input).toBeVisible();
    await input.press("ArrowRight"); await expect(input).toHaveCount(0);
  });

  test(`formulas commit only explicit completion and compact controls use shared styling on ${url}`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 808 });
    const editor = await fresh(page, url);
    await page.keyboard.type("Before $"); await page.keyboard.press("Tab");
    const input = page.getByRole("textbox", { name: "行内公式 LaTeX 源码", exact: true });
    await input.fill("x^2"); await page.getByRole("button", { name: "完成", exact: true }).click();
    const formula = page.locator(".inline-equation-preview");
    for (const exit of ["escape", "outside"] as const) {
      await formula.click(); await input.fill("unsaved");
      await expect(formula).toHaveAttribute("aria-label", "行内公式 x^2，点击编辑");
      if (exit === "escape") await input.press("Escape");
      else await page.getByLabel("笔记标题", {exact: true}).click();
      await expect(input).toHaveCount(0);
      await formula.click(); await expectEquationSource(input, "x^2");
      await input.fill(""); await input.press("Escape");
      await expect(formula).toHaveAttribute("aria-label", "行内公式 x^2，点击编辑");
    }
    await formula.click(); await input.fill("y^2"); await input.press("Enter");
    await expect(formula).toHaveAttribute("aria-label", "行内公式 y^2，点击编辑");
    await page.keyboard.press("Enter"); await page.keyboard.type("$$"); await page.keyboard.press("Tab");
    const block = page.locator(".note-formula-disclosure");
    const blockInput = page.getByRole("textbox", { name: "公式 LaTeX 源码", exact: true });
    await block.click(); await blockInput.fill("a+b"); await blockInput.press("Enter");
    await block.click(); await blockInput.fill("unsaved"); await blockInput.press("Escape");
    await block.click(); await expectEquationSource(blockInput, "a+b");
    await blockInput.fill("also unsaved"); await page.getByLabel("笔记标题", {exact: true}).click();
    await block.click(); await expectEquationSource(blockInput, "a+b"); await blockInput.press("Escape");
    await editor.click(); await page.keyboard.press("ControlOrMeta+End"); await page.keyboard.press("Enter");
    await page.keyboard.type("/");
    const menu = page.getByRole("listbox");
    await expect(menu.getByText("#", { exact: true })).toBeVisible();
    await expect(menu.getByText("用于顶级标题", { exact: true })).toHaveCount(0);
    const badge = menu.locator('[data-position="right"]').first();
    await expect(badge).toHaveCSS("box-shadow", "none"); await expect(badge).toHaveCSS("border-top-width", "0px");
    await expect.poll(() => menu.evaluate(el => {
      for (let node: Element | null = el; node; node = node.parentElement) {
        if (Number(getComputedStyle(node).opacity) < 1) return false;
      }
      return true;
    })).toBe(true);
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/notes-compact-suggestion.png" });
    await page.keyboard.press("Escape"); await page.keyboard.press("Backspace");
    await page.keyboard.type("Select this text"); await page.keyboard.press("ControlOrMeta+a");
    const toolbar = page.getByRole("toolbar").filter({ has: page.locator(".bn-ak-button") });
    await expect(toolbar).toBeVisible();
    await expect(toolbar.locator(".bn-ak-button").first()).toHaveCSS("height", "28px");
    await expect(toolbar.locator(".bn-ak-button").first()).toHaveCSS("font-size", "13px");
    await expect.poll(() => toolbar.evaluate(el => {
      for (let node: Element | null = el; node; node = node.parentElement) {
        if (Number(getComputedStyle(node).opacity) < 1) return false;
      }
      return true;
    })).toBe(true);
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/notes-compact-toolbar.png" });
  });
}
