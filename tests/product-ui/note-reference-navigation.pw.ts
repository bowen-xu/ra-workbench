import { expect, test, type Page } from "@playwright/test";
async function fresh(page: Page, title: string) {
  const editor = page.locator(".notes-content .bn-editor");
  const previous = await editor.count() ? await editor.elementHandle() : null;
  await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
  if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
  await page.getByRole("textbox", { name: "笔记标题" }).fill(title); await editor.click();
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  return editor;
}
for (const url of ["/", "/?preview=app"]) {
  test(`references hover, select atomically and navigate by pointer or keyboard on ${url}`, async ({ page }) => {
    await page.goto(url); await page.getByRole("button", { name: "笔记", exact: true }).click();
    await fresh(page, "Reference destination");
    const editor = await fresh(page, "Reference navigation");
    await page.keyboard.type("Before @Reference destination"); await page.keyboard.press("Tab");
    const reference = editor.getByRole("link", { name: "Reference destination", exact: true });
    await expect(reference).toHaveAttribute("contenteditable", "false");
    const textColor = await reference.evaluate(element => getComputedStyle(element).color);
    await reference.hover();
    await expect(reference).toHaveCSS("cursor", "pointer");
    await expect(reference).toHaveCSS("background-color", "rgb(240, 239, 237)");
    await expect(reference).toHaveCSS("color", textColor);
    await expect(reference.locator(".inline-reference-label")).toHaveCSS("text-decoration-style", "solid");
    await reference.evaluate(element => {
      const wrapper = element.closest(".node-researchMention")!;
      const range = document.createRange(); range.setStartAfter(wrapper); range.collapse(true);
      const selection = document.getSelection()!; selection.removeAllRanges(); selection.addRange(range);
    });
    await page.keyboard.press("ArrowLeft");
    await expect(editor.locator(".node-researchMention.ProseMirror-selectednode")).toHaveCount(1);
    await expect.poll(() => page.evaluate(() => document.getSelection()?.toString())).toBe("Reference destination");
    await page.keyboard.press("ArrowLeft"); await page.keyboard.press("ArrowRight");
    await expect(editor.locator(".node-researchMention.ProseMirror-selectednode")).toHaveCount(1);
    await page.keyboard.press("Delete"); await expect(reference).toHaveCount(0);
    await page.keyboard.press("ControlOrMeta+z"); await expect(reference).toHaveText("Reference destination");
    await reference.click(); await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("Reference destination");
    await page.getByRole("tab", { name: "全部笔记", exact: true }).click();
    await page.locator(".reading-note-list").getByRole("button", { name: /^独立笔记 Reference navigation/ }).click();
    await reference.focus(); await reference.press("Enter");
    await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("Reference destination");
    if (url.includes("preview")) {
      await fresh(page, "Paper reference navigation");
      await page.keyboard.type("@Synthetic Research Preview");
      await page.getByRole("listbox").getByRole("option", { name: /^Synthetic Research Preview/ }).click();
      const paper = editor.getByRole("link", { name: "Synthetic Research Preview: Evidence and Autonomy", exact: true });
      await paper.click();
      await expect(page.getByRole("row", { name: /Synthetic Research Preview/ })).toBeVisible();
    }
  });
}
