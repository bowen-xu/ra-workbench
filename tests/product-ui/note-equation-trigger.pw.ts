import { expect, test, type Page } from "@playwright/test";

async function newPage(page: Page, title: string) {
  const editors = page.locator(".notes-content .bn-editor");
  const previous = await editors.count() ? await editors.elementHandle() : null;
  await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
  if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
  await expect(editors).toBeVisible();
  await page.getByRole("textbox", { name: "笔记标题" }).fill(title);
  const editor = page.locator(".notes-content .bn-editor");
  await editor.click();
  return editor;
}

for (const url of ["/", "/?preview=app"]) {
  test(`dollar suggestions switch between inline and block equations without premature insertion on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    const editor = await newPage(page, "Dollar equation acceptance");
    await page.keyboard.type("text $");
    const menu = page.getByRole("listbox");
    await expect(menu.getByRole("option")).toHaveCount(1);
    await expect(menu.getByText("行内公式", { exact: true })).toBeVisible();
    await expect(editor.locator(".inline-equation-preview")).toHaveCount(0);
    await page.keyboard.press("Tab");
    const inlineInput = page.getByRole("textbox", { name: "行内公式 LaTeX 源码", exact: true });
    await expect(inlineInput).toBeFocused();
    await inlineInput.fill("x^2"); await inlineInput.press("Enter");
    await page.keyboard.type("After");
    await expect(editor.locator(".bn-block-content")).toHaveCount(1);

    await expect(editor.locator(".bn-block-content")).toContainText("After");
    await expect(editor).not.toContainText("$");
    await page.keyboard.press("Enter");
    await page.keyboard.type("$");
    await expect(menu.getByText("行内公式", { exact: true })).toBeVisible();
    await page.keyboard.type("$");
    await expect(menu.getByRole("option")).toHaveCount(1);
    await expect(menu.getByText("公式块", { exact: true })).toBeVisible();
    await expect(menu.getByText("行内公式", { exact: true })).toHaveCount(0);
    await expect(editor.locator('[data-content-type="formulaBlock"]')).toHaveCount(0);
    await page.keyboard.press("Backspace");
    await expect(menu.getByText("行内公式", { exact: true })).toBeVisible();
    await page.keyboard.type("$");
    await expect(menu.getByText("公式块", { exact: true })).toBeVisible();
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/note-dollar-equation-menu.png" });
    await menu.getByText("公式块", { exact: true }).click();
    await expect(menu).toHaveCount(0);
    await expect(editor.locator('[data-content-type="formulaBlock"]')).toHaveCount(1);
    await expect(editor.locator(".inline-equation-preview")).toHaveCount(1);
    await expect(editor).not.toContainText("$");
    await page.getByRole("button", { name: "空公式，点击编辑", exact: true }).click();
    const blockInput = page.getByRole("textbox", { name: "公式 LaTeX 源码", exact: true });
    await expect(blockInput).toBeFocused(); await blockInput.fill("E=mc^2"); await blockInput.press("Enter");
    await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
    await page.reload();
    if (!await page.locator(".notes-content").count()) await page.getByRole("button", { name: "笔记", exact: true }).click();
    await page.getByRole("tab", { name: "全部笔记", exact: true }).click();
    await page.locator(".reading-note-list").getByRole("button", { name: /^独立笔记 Dollar equation acceptance/ }).click();
    await expect(editor.locator(".inline-equation-preview")).toHaveCount(1);
    await expect(editor.locator('[data-content-type="formulaBlock"]')).toHaveCount(1);
    await expect(page.getByRole("button", { name: "公式 E=mc^2，点击编辑", exact: true })).toBeVisible();
  });

  test(`dollar menus cancel into literal text and leave code unchanged on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    const editor = await newPage(page, "Literal dollar acceptance");
    await page.keyboard.type("$$");
    await expect(page.getByRole("listbox").getByText("公式块", { exact: true })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("listbox")).toHaveCount(0);
    await expect(editor.locator(".bn-block-content")).toHaveText("$$");
    for (const dollars of ["$", "$$"]) {
      await newPage(page, `Space after ${dollars}`);
      await page.keyboard.type(dollars);
      await expect(page.getByRole("listbox")).toBeVisible();
      await page.keyboard.press("Space");
      await expect(page.getByRole("listbox")).toHaveCount(0);
      expect(await editor.locator(".bn-block-content").textContent()).toBe(`${dollars} `);
      await expect(editor.locator(".inline-equation-preview, [data-content-type='formulaBlock']")).toHaveCount(0);
    }
    await page.keyboard.press("Enter"); await page.keyboard.type("$100");
    await expect(page.getByRole("listbox")).toHaveCount(0);
    await expect(editor).toContainText("$100");
    await expect(editor.locator(".inline-equation-preview, [data-content-type='formulaBlock']")).toHaveCount(0);
    await page.keyboard.press("Enter"); await page.keyboard.type("Before$/@");
    await expect(page.getByRole("listbox")).toHaveCount(0);
    await expect(editor).toContainText("Before$/@");
    await newPage(page, "Dollar code acceptance");
    await page.keyboard.type("/code");
    await page.getByRole("listbox").getByText("代码块", { exact: true }).click();
    await page.keyboard.type("$$");
    await expect(page.getByRole("listbox")).toHaveCount(0);
    await expect(editor.locator('[data-content-type="codeBlock"]')).toContainText("$$");
    await newPage(page, "Dollar inline code acceptance");
    await page.keyboard.press("ControlOrMeta+e"); await page.keyboard.type("$");
    await expect(page.getByRole("listbox")).toHaveCount(0);
    await expect(editor.locator("code")).toHaveText("$");
  });

  test(`slash queries allow one space and close on the second on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    const editor = await newPage(page, "Slash space limit");
    await page.keyboard.type("/inline equation");
    await expect(page.getByRole("listbox").getByText("行内公式", { exact: true })).toBeVisible();
    await page.keyboard.press("Space");
    await expect(page.getByRole("listbox")).toHaveCount(0);
    expect(await editor.locator(".bn-block-content").textContent()).toBe("/inline equation ");
    await newPage(page, "Consecutive slash spaces");
    await page.keyboard.type("/"); await page.keyboard.press("Space");
    await expect(page.getByRole("listbox")).toBeVisible();
    await page.keyboard.press("Space");
    await expect(page.getByRole("listbox")).toHaveCount(0);
    expect(await editor.locator(".bn-block-content").textContent()).toBe("/  ");
  });
}
