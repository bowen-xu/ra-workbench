import { equationSource, expectEquationSource } from "../ui/equation-source";
import { expect, test, type Locator, type Page } from "@playwright/test";

const sourceName = "公式 LaTeX 源码";
async function paneMetrics(page: Page) {
  return page.locator(".notes-content").evaluate(pane => ({
    scrollTop: pane.scrollTop, scrollHeight: pane.scrollHeight,
    height: pane.getBoundingClientRect().height,
  }));
}

async function reopenFromInnerPreviewAndDismiss(page: Page, preview: Locator) {
  const formula = page.locator(".note-formula-disclosure");
  const input = page.getByRole("textbox", { name: sourceName, exact: true });
  const before = await paneMetrics(page);
  // Keep both callers aimed at nested SPANs (empty label / KaTeX symbol).
  // Clicking formula.click() at the button's blank center misses this bug:
  // ProseMirror can focus its editor on mouseup before Ariakit captures the
  // disclosure. See docs/testing/note-formula-popover-regression.md.
  await preview.click(); await expect(input).toBeFocused();
  // aria-expanded stays tied to the real button only if Ariakit has not
  // overwritten its disclosure with the surrounding contenteditable editor.
  await expect(formula).toHaveAttribute("aria-expanded", "true");
  await preview.click(); await expect(input).toHaveCount(0);
  await preview.click(); await expect(input).toBeFocused();
  await expect(formula).toHaveAttribute("aria-expanded", "true");
  // Keep the paragraph clear of the popup when it flips above the formula.
  await page.locator(".notes-content .bn-editor").getByText("Before 19", { exact: true }).click();
  await expect(input).toHaveCount(0);
  await expect(page.locator(".notes-content .bn-editor")).toBeFocused();
  expect(await paneMetrics(page)).toEqual(before);
}

for (const url of ["/", "/?preview=app"]) {
  test(`formula editing floats without changing page length or scroll and persists on ${url}`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 808 });
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    const previous = await page.locator(".notes-content .bn-editor").count()
      ? await page.locator(".notes-content .bn-editor").elementHandle() : null;
    await page.locator(".notes-explorer").getByRole("button", { name: "新建独立笔记", exact: true }).click();
    if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
    await page.getByLabel("笔记标题", {exact: true}).fill("公式交互测试");
    const editor = page.locator(".notes-content .bn-editor");
    await editor.click();
    for (let index = 0; index < 24; index++) {
      await page.keyboard.type(`Before ${index}`); await page.keyboard.press("Enter");
    }
    await page.keyboard.type("/");
    await page.getByRole("listbox").getByText("公式块", { exact: true }).click();
    const formula = page.locator(".note-formula-disclosure");
    const input = page.getByRole("textbox", { name: sourceName, exact: true });
    await expect(input).toHaveCount(0);
    await expect(formula).toHaveText("TEX添加 TeX 公式");
    await editor.getByText("Before 23", { exact: true }).click();
    await expect(formula).toHaveCSS("background-color", "rgb(248, 247, 246)");
    await expect(formula).toHaveCSS("padding-top", "8px");
    await expect(formula).toHaveCSS("padding-bottom", "8px");
    await expect(formula.locator(".katex")).toHaveCount(0);
    await formula.scrollIntoViewIfNeeded();
    const emptyIdle = await paneMetrics(page);
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/note-formula-empty.png" });
    await reopenFromInnerPreviewAndDismiss(page, formula.getByText("添加 TeX 公式", { exact: true }));
    await formula.click();
    await expect(input).toBeFocused();
    await expect(formula.getByText("添加 TeX 公式", { exact: true })).toBeVisible();
    const editingOuter = page.locator(".bn-block-outer").filter({ has: formula }).last();
    const selectedWhileEditing = await editingOuter.evaluate(node => getComputedStyle(node).backgroundColor === "rgb(234, 242, 237)");
    await expect(formula).toHaveCSS("background-color", "rgb(248, 247, 246)");
    if (selectedWhileEditing) await expect(formula).toHaveCSS("background-image", /rgba\(63, 109, 80, 0\.1\)/);
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/note-formula-empty-editing.png" });
    expect(await paneMetrics(page)).toEqual(emptyIdle);
    await expect.poll(async () => {
      const anchor = await formula.boundingBox();
      const panel = await page.getByRole("dialog", { name: "编辑公式", exact: true }).boundingBox();
      if (!anchor || !panel) return false;
      const gap = panel.y >= anchor.y + anchor.height ? panel.y - anchor.y - anchor.height : anchor.y - panel.y - panel.height;
      return Math.abs(gap - 10) < 1;
    }).toBe(true);
    await input.fill(" \n ");
    await page.getByRole("button", { name: "完成", exact: true }).click();
    await expect(input).toHaveCount(0);
    await expect(formula).toHaveText("TEX添加 TeX 公式");
    const emptyOuter = page.locator(".bn-block-outer").filter({ has: formula }).last();
    await expect(emptyOuter).toHaveCSS("background-color", "rgb(234, 242, 237)");
    await expect(formula).toHaveCSS("background-color", "rgb(248, 247, 246)");
    await expect(formula).toHaveCSS("background-image", /rgba\(63, 109, 80, 0\.1\)/);
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/formula-empty-selected-background.png" });
    expect(await paneMetrics(page)).toEqual(emptyIdle);
    await formula.click(); await expect(input).toBeFocused();
    await input.fill("E=mc^2");
    await expect(formula).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
    await expect(formula.locator(".note-formula-preview")).toHaveAttribute("aria-label", "E=mc^2");
    await page.getByRole("button", { name: "完成", exact: true }).click();
    await expect(input).toHaveCount(0);
    await expect(formula).toBeFocused();
    const outer = page.locator(".bn-block-outer").filter({ has: formula }).last();
    await expect(outer).toHaveCSS("background-color", "rgb(234, 242, 237)");
    await expect(outer).toHaveCSS("outline-style", "none");
    await expect(formula).toHaveCSS("outline-style", "none");
    await expect(page.locator(".note-formula-block")).toHaveCSS("outline-style", "none");
    await reopenFromInnerPreviewAndDismiss(page, formula.locator(".katex-html .mord").first());
    const blockCount = await editor.locator(".bn-block[data-id]").count();
    await formula.focus(); await page.keyboard.press("Enter");
    await expect(input).toBeFocused();
    expect(await editor.locator(".bn-block[data-id]").count()).toBe(blockCount);
    await input.press("Escape"); await expect(input).toHaveCount(0);
    await editor.focus(); await page.keyboard.press("Enter");
    await expect(input).toBeFocused();
    expect(await editor.locator(".bn-block[data-id]").count()).toBe(blockCount);
    await input.press("Escape"); await expect(input).toHaveCount(0);
    await formula.hover();
    await page.getByRole("button", { name: "添加块", exact: true }).click();
    await page.keyboard.press("Escape");
    for (let index = 0; index < 24; index++) {
      await page.keyboard.type(`After ${index}`); await page.keyboard.press("Enter");
    }
    await formula.scrollIntoViewIfNeeded();
    const idle = await paneMetrics(page);
    await formula.click(); await expect(input).toBeFocused();
    expect(await paneMetrics(page)).toEqual(idle);
    expect(await page.getByRole("dialog", { name: "编辑公式", exact: true }).evaluate(panel =>
      panel.closest(".bn-editor") === null && getComputedStyle(panel.parentElement!).position === "fixed")).toBe(true);
    await input.fill("E=mc^2\\,");
    expect(await paneMetrics(page)).toEqual(idle);
    // Put the disclosure above the visible pane: default focus restoration
    // would scroll it back into view when Done or Enter closes the popover.
    await page.locator(".notes-content").evaluate(pane => { pane.scrollTop += 400; });
    const scrolled = await paneMetrics(page);
    expect(scrolled.scrollTop).toBeGreaterThan(idle.scrollTop + 300);
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/note-formula-floating.png" });
    await page.getByRole("button", { name: "完成", exact: true }).click();
    await expect(input).toHaveCount(0); await expect(formula).toBeFocused();
    expect(await paneMetrics(page)).toEqual(scrolled);
    await formula.scrollIntoViewIfNeeded(); await formula.click();
    await expect(input).toBeFocused();
    const reopened = await paneMetrics(page);
    await input.dispatchEvent("keydown", { key: "Enter", code: "Enter", keyCode: 229, isComposing: true });
    await expect(input).toBeVisible();
    await input.press("Shift+Enter");
    await expectEquationSource(input, /\n/);
    expect((await equationSource(input)).replace("\n", "")).toBe("E=mc^2\\,");
    await input.fill("E=mc^2\\,");
    await input.press("Enter"); await expect(input).toHaveCount(0);
    expect(await paneMetrics(page)).toEqual(reopened);
    await formula.click(); await expectEquationSource(input, "E=mc^2\\,");
    await input.press("Escape"); await expect(input).toHaveCount(0);
    await formula.click(); await expect(input).toBeFocused();
    await page.getByLabel("笔记标题", {exact: true}).click();
    await expect(input).toHaveCount(0);
    await expect(page.getByLabel("笔记标题", {exact: true})).toBeFocused();
    await expect(outer).toHaveCSS("background-color", "rgb(234, 242, 237)");
    await formula.scrollIntoViewIfNeeded(); await formula.click(); await expect(input).toBeFocused();
    const outsideMetrics = await paneMetrics(page);
    const paneBounds = await page.locator(".notes-content").boundingBox();
    const formulaBounds = await formula.boundingBox();
    await page.mouse.click(paneBounds!.x + 5, formulaBounds!.y + formulaBounds!.height / 2);
    await expect(input).toHaveCount(0);
    await expect(outer).toHaveCSS("background-color", "rgb(234, 242, 237)");
    expect(await paneMetrics(page)).toEqual(outsideMetrics);
    await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
    await page.reload();
    if (!await page.locator(".notes-content").count()) await page.getByRole("button", { name: "笔记", exact: true }).click();
    await page.getByRole("tab", { name: "全部笔记", exact: true }).click();
    await page.locator(".reading-note-list").getByRole("button", { name: /^独立笔记 公式交互测试/ }).click();
    await expect(input).toHaveCount(0);
    await formula.click(); await expectEquationSource(input, "E=mc^2\\,");
    await input.press("Escape");
    await page.setViewportSize({ width: 375, height: 667 });
    await formula.click(); await expect(input).toBeFocused();
    const panel = await page.getByRole("dialog", { name: "编辑公式", exact: true }).boundingBox();
    expect(panel!.x).toBeGreaterThanOrEqual(0);
    expect(panel!.x + panel!.width).toBeLessThanOrEqual(375);
  });
}
