import { expect, test, type Page } from "@playwright/test";

async function openWriting(page: Page, platform: string): Promise<void> {
  await page.addInitScript((value) => {
    Object.defineProperty(navigator, "platform", { get: () => value });
  }, platform);
  await page.goto("/");
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await page.getByRole("button", { name: "新建稿件", exact: true }).click();
  await page.getByRole("dialog", { name: "新建 LaTeX 稿件" }).getByRole("textbox").fill("快捷键回归");
  await page.getByRole("button", { name: "创建稿件", exact: true }).click();
  await expect(page.locator(".writing-editor-host .cm-content")).toBeVisible();
}

async function replaceSource(page: Page, modifier: string, text: string): Promise<void> {
  await page.locator(".writing-editor-host .cm-content").click();
  await page.keyboard.press(`${modifier}+a`);
  await page.keyboard.insertText(text);
}

for (const [platform, modifier] of [["MacIntel", "Meta"], ["Win32", "Control"]] as const) {
  test.describe(platform, () => {
    test.beforeEach(async ({ page }) => openWriting(page, platform));

    test("Tab accepts completion; save/compile shortcut retains source without a toolchain", async ({ page }) => {
      await replaceSource(page, modifier, "\\sec");
      await page.keyboard.press("Control+Space");
      await expect(page.locator(".cm-tooltip-autocomplete")).toBeVisible();
      await page.keyboard.press("Tab");
      await expect(page.locator(".cm-line")).toHaveText("\\section{}");
      await page.keyboard.press(`${modifier}+s`);
      await expect(page.locator(".save-state")).toHaveText("已保存");
      await expect(page.getByRole("heading", { name: "LaTeX 环境不可用" })).toBeVisible();
      // Workspace switches retain the live editor. Durable restart recovery
      // belongs to the desktop adapter tests.
      await page.getByRole("button", { name: "笔记", exact: true }).click();
      await page.getByRole("button", { name: "写作", exact: true }).click();
      await expect(page.locator(".cm-line")).toHaveText("\\section{}");
    });

    test("style, duplicate, delete, undo and line navigation use Overleaf bindings", async ({ page }) => {
      await replaceSource(page, modifier, "first\nsecond");
      await page.keyboard.press(`${modifier}+Shift+L`);
      const lineInput = page.locator('.cm-dialog input[name="line"]');
      await expect(lineInput).toBeVisible();
      await lineInput.fill("1");
      await lineInput.press("Enter");
      await page.keyboard.press(`${modifier}+d`);
      await expect(page.locator(".cm-line")).toHaveText("second");
      await page.keyboard.press(`${modifier}+z`);
      await expect(page.locator(".cm-line")).toHaveText(["first", "second"]);
      await replaceSource(page, modifier, "word");
      await page.keyboard.press(`${modifier}+a`);
      await page.keyboard.press(`${modifier}+b`);
      await expect(page.locator(".cm-line")).toHaveText("\\textbf{word}");
      await page.keyboard.press(`${modifier}+b`);
      await page.keyboard.press(`${modifier}+i`);
      await expect(page.locator(".cm-line")).toHaveText("\\textit{word}");
      await page.keyboard.press(`${modifier}+i`);
      await page.keyboard.press(`${modifier}+Shift+D`);
      await expect(page.locator(".cm-line")).toHaveText("wordword");
      await page.keyboard.press("Control+u");
      await expect(page.locator(".cm-line")).toHaveText("wordWORD");
      await page.keyboard.press("Control+Shift+U");
      await expect(page.locator(".cm-line")).toHaveText("wordword");
    });

    test("multiple cursors edit both lines", async ({ page }) => {
      await replaceSource(page, modifier, "first\nsecond");
      await page.keyboard.press(`${modifier}+Alt+ArrowUp`);
      await page.keyboard.insertText("!");
      await expect(page.locator(".cm-line")).toHaveText(["first!", "second!"]);
    });
  });
}

test("Code Check marks LaTeX while typing and its compile-menu switch clears findings", async ({ page }) => {
  await openWriting(page, "MacIntel");
  await replaceSource(page, "Meta", "text_a");
  await expect(page.locator(".writing-editor-status")).toContainText("Code Check 发现问题");
  await expect(page.locator(".cm-lintRange-error")).toBeVisible();

  const trigger = page.getByRole("button", { name: "编译选项", exact: true });
  await trigger.click();
  await page.getByRole("group", { name: "Code Check" }).getByRole("menuitemradio", { name: "关闭", exact: true }).click();
  await expect(page.locator(".writing-editor-status")).not.toContainText("Code Check 发现问题");
  await expect(page.locator(".cm-lintRange-error")).toHaveCount(0);

  await trigger.click();
  await page.getByRole("group", { name: "Code Check" }).getByRole("menuitemradio", { name: "开启", exact: true }).click();
  await replaceSource(page, "Meta", "%%novalidate\ntext_a");
  await expect(page.locator(".writing-editor-status")).not.toContainText("Code Check 发现问题");
  await expect(page.locator(".cm-lintRange-error")).toHaveCount(0);
});


test("file and explorer menu actions survive non-focusing mouse presses", async ({ page }) => {
  await openWriting(page, "MacIntel");
  // Reproduce WebKit's pointer -> null-target blur -> click ordering in Chromium.
  await page.evaluate(() => document.addEventListener("mousedown", (event) => {
    if (!(event.target instanceof Element) || !event.target.closest('[role^="menuitem"]')) return;
    event.preventDefault();
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  }, true));
  const menu = page.locator(".writing-toolbar .writing-menu");
  await menu.locator('[aria-haspopup="menu"]').click();
  await page.getByRole("menuitem", { name: "保存当前文件", exact: true }).click();
  await expect(menu.locator('[aria-haspopup="menu"]')).toHaveAttribute("aria-expanded", "false");
  await menu.locator('[aria-haspopup="menu"]').click();
  await page.getByRole("menuitem", { name: "命令面板", exact: true }).click();
  await expect(page.getByRole("dialog", { name: "LaTeX 写作命令" })).toBeVisible();
  await expect(page.getByRole("textbox", { name: "搜索写作命令" })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(page.getByRole("dialog", { name: "LaTeX 写作命令" })).toBeHidden();
  await expect(menu.locator('[aria-haspopup="menu"]')).toBeFocused();
  const file = page.locator(".writing-file-menu").first();
  await file.locator("..").hover();
  await file.locator('[aria-haspopup="menu"]').click();
  await page.getByRole("menuitem", { name: "重命名", exact: true }).click();
  await expect(page.getByRole("textbox", { name: "工程相对路径" })).toBeFocused();
  await expect(file.locator('[aria-haspopup="menu"]')).toHaveAttribute("aria-expanded", "false");
});


test("menu dismissal and keyboard focus work across title, layout and file controls", async ({ page }) => {
  await openWriting(page, "MacIntel");
  for (const name of ["文件", "布局", "稿件菜单：快捷键回归", "文件操作 main.tex"]) {
    const trigger = page.getByRole("button", { name, exact: true });
    if (name.startsWith("文件操作")) await page.locator(".writing-file-row").filter({has: trigger}).hover();
    await trigger.click();
    await expect(trigger).toHaveAttribute("aria-expanded", "true");
    // Menus may cover the editor's center; dismiss through exposed source text.
    await page.locator(".writing-editor-host .cm-content").click({ position: { x: 100, y: 8 } });
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
    await trigger.focus();
    await page.keyboard.press("ArrowDown");
    await expect(page.locator('[role="menu"]:visible [role^="menuitem"]:not(:disabled)').first()).toBeFocused();
    await page.keyboard.press("End");
    await page.keyboard.press("Escape");
    await expect(trigger).toBeFocused();
    await expect(trigger).toHaveAttribute("aria-expanded", "false");
  }
});

test("shared editor search keeps borders and keyboard focus without extra frames", async ({page}) => {
  await openWriting(page, "MacIntel");
  await replaceSource(page, "Meta", "focus target and focus target");
  await page.keyboard.press("Meta+f");
  const panel = page.locator(".text-search-panel");
  const find = panel.getByRole("textbox", {name: "查找内容", exact: true});
  await expect(find).toBeFocused();
  await find.fill("focus");
  const close = panel.getByRole("button", {name: "关闭查找", exact: true});
  await close.focus();
  const field = find.locator("..");
  const border = await field.evaluate(element => getComputedStyle(element).borderColor);
  await expect(close).toHaveCSS("outline-style", "none");
  await find.focus();
  await expect(find).toHaveCSS("outline-style", "none");
  await expect(field).toHaveCSS("outline-style", "none");
  await expect(field).toHaveCSS("border-color", border);
  await expect(field).toHaveCSS("box-shadow", "none");
  await find.press("Escape");
  await expect(panel).toHaveCount(0);
  await expect(page.locator(".writing-editor-host .cm-content")).toBeFocused();
});
