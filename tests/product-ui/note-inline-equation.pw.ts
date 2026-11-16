import { expectEquationSource } from "../ui/equation-source";
import { expect, test, type Page } from "@playwright/test";

async function expectTitleCaret(page: Page): Promise<void> {
  await expect(page.locator(".notes-content .bn-editor")).toBeFocused();
  await expect.poll(() => page.getByLabel("笔记标题", {exact: true})
    .evaluate(title => title.contains(document.getSelection()?.anchorNode ?? null))).toBe(true);
}

for (const url of ["/", "/?preview=app"]) {
  for (const exit of ["done", "enter", "escape", "outside"] as const) {
    test(`empty equations disappear on ${exit} while adjacent text and another equation survive on ${url}`, async ({ page }) => {
      await page.goto(url);
      await page.getByRole("button", { name: "笔记", exact: true }).click();
      const input = page.getByRole("textbox", { name: "行内公式 LaTeX 源码", exact: true });
      const previous = await page.locator(".notes-content .bn-editor").count()
        ? await page.locator(".notes-content .bn-editor").elementHandle() : null;
      await page.locator(".notes-explorer").getByRole("button", { name: "新建独立笔记", exact: true }).click();
      if (previous !== null) await expect.poll(() => previous.evaluate(element => element.isConnected)).toBe(false);
      await page.getByLabel("笔记标题", {exact: true}).fill(`Empty equation ${exit}`);
      const editor = page.locator(".notes-content .bn-editor");
      await editor.click(); await page.keyboard.type("Before /inline equation");
      await page.getByRole("listbox").getByText("行内公式", { exact: true }).click();
      await expect(input).toBeFocused(); await input.fill("x^2"); await input.press("Enter");
      await expect(input).toHaveCount(0);
      await page.keyboard.type("Middle /inline equation");
      await page.getByRole("listbox").getByText("行内公式", { exact: true }).click();
      await expect(page.getByRole("listbox")).toHaveCount(0);
      await expect(input).toBeFocused();
      const empty = page.locator(".inline-equation-preview.empty");
      await expect(empty.getByText("新公式", { exact: true })).toBeVisible();
      await expect(empty.locator(".inline-equation-placeholder-symbol .katex-html")).toContainText("x");
      await expect(empty.locator(".inline-equation-placeholder-symbol msqrt")).toHaveCount(1);
      await expect(empty).toHaveCSS("background-color", "rgb(240, 239, 237)");
      await expect(input).toHaveAttribute("aria-placeholder", "E = mc^2");
      if (url.includes("preview") && exit === "done") await page.screenshot({ path: ".tmp/inline-equation-empty.png" });
      if (exit !== "enter") await input.fill("   ");
      if (exit === "done") await page.getByRole("button", { name: "完成", exact: true }).click();
      if (exit === "enter") await input.press("Enter");
      if (exit === "escape") await input.press("Escape");
      if (exit === "outside") await page.getByLabel("笔记标题", {exact: true}).click();
      await expect(input).toHaveCount(0); await expect(empty).toHaveCount(0);
      await expect(page.locator(".inline-equation-preview")).toHaveCount(1);
      await expect(page.getByRole("button", { name: "行内公式 x^2，点击编辑", exact: true })).toBeVisible();
      const paragraph = editor.locator('.bn-block-content:not([data-content-type="documentTitle"])');
      await expect(paragraph).toHaveCount(1); await expect(paragraph).toContainText("Before");
      await expect(paragraph).toContainText("Middle");
      if (exit === "outside") await expectTitleCaret(page);
      else await expect(editor).toBeFocused();
      await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
      await page.reload();
      await page.getByRole("tab", { name: `Empty equation ${exit}`, exact: true }).click();
      await expect(page.locator(".inline-equation-preview.empty")).toHaveCount(0);
      const kept = page.getByRole("button", { name: "行内公式 x^2，点击编辑", exact: true });
      await kept.click(); await expectEquationSource(input, "x^2");
      await input.fill(""); await input.press("Enter");
      await expect(page.locator(".inline-equation-preview")).toHaveCount(0);
      await expect(editor.locator('.bn-block-content:not([data-content-type="documentTitle"])')).toContainText("Middle");
      await page.keyboard.press("ControlOrMeta+z");
      // Clearing source and deleting the empty node belong to the same native
      // history group; undo restores the original nonempty equation.
      await expect(kept).toBeVisible();
      await kept.click(); await expectEquationSource(input, "x^2");
      await input.fill("y^2"); await input.press("Enter");
      await expect(page.getByRole("button", { name: "行内公式 y^2，点击编辑", exact: true })).toBeVisible();
    });
  }

  test(`inline equation inserts and edits within a paragraph with durable source on ${url}`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 808 });
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await page.locator(".notes-explorer").getByRole("button", { name: "新建独立笔记", exact: true }).click();
    await page.getByLabel("笔记标题", {exact: true}).fill("Inline equation acceptance");
    const editor = page.locator(".notes-content .bn-editor");
    await editor.click(); await page.keyboard.type("Before /inline equation");
    await page.getByRole("listbox").getByText("行内公式", { exact: true }).click();
    await expect(page.getByRole("listbox")).toHaveCount(0);
    const input = page.getByRole("textbox", { name: "行内公式 LaTeX 源码", exact: true });
    const formula = page.locator(".inline-equation-preview");
    await expect(input).toBeFocused();
    const paragraph = editor.locator('.bn-block-content:not([data-content-type="documentTitle"])').first();
    await expect(paragraph).toContainText("Before");
    await expect(editor.locator('[data-content-type="formulaBlock"]')).toHaveCount(0);
    await input.fill("E=mc^2");
    await expect(formula.locator(".katex")).toBeVisible();
    await input.dispatchEvent("keydown", { key: "Enter", code: "Enter", keyCode: 229, isComposing: true });
    await expect(input).toBeVisible();
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/inline-equation-editing.png" });
    await input.press("Enter"); await expect(input).toHaveCount(0);
    await expect(editor).toBeFocused();
    await page.keyboard.type("After");
    await expect(paragraph).toContainText("Before"); await expect(paragraph).toContainText("After");
    await expect(editor.locator('.bn-block-content:not([data-content-type="documentTitle"])')).toHaveCount(1);
    await expect(formula).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
    const before = await paragraph.boundingBox();
    await formula.click(); await expectEquationSource(input, "E=mc^2");
    expect(await paragraph.boundingBox()).toEqual(before);
    await input.fill("x^2+y^2");
    await input.press("Escape"); await expect(input).toHaveCount(0);
    await expect(formula).toBeFocused();
    await page.keyboard.press("Enter"); await expect(input).toBeFocused();
    await expectEquationSource(input, "E=mc^2");
    await input.fill("x^2+y^2");
    await page.getByRole("button", { name: "完成", exact: true }).click();
    await expect(input).toHaveCount(0); await expect(editor).toBeFocused();
    await formula.click(); await expect(input).toBeVisible();
    await page.getByLabel("笔记标题", {exact: true}).click();
    await expect(input).toHaveCount(0);
    await expectTitleCaret(page);
    await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
    await page.reload();
    await page.getByRole("tab", { name: "Inline equation acceptance", exact: true }).click();
    await expect(input).toHaveCount(0);
    await formula.click(); await expectEquationSource(input, "x^2+y^2");
    await page.setViewportSize({ width: 375, height: 667 });
    const panel = page.getByRole("dialog", { name: "编辑行内公式", exact: true });
    await expect.poll(() => panel.evaluate(element => {
      const rect = element.getBoundingClientRect();
      return rect.left >= 0 && rect.right <= innerWidth && rect.top >= 0 && rect.bottom <= innerHeight;
    })).toBe(true);
  });
}
