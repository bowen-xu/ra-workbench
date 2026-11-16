import { expect, test } from "@playwright/test";

test("new source files share the selected tab style with existing files", async ({ page }) => {
  await page.goto("/?demo=writing-editor-diff");
  await page.getByRole("combobox", { name: "模拟文件场景", exact: true }).selectOption("mixed");
  const pane = page.getByRole("region", { name: "编辑器 Diff 预览", exact: true });
  const main = pane.locator(".editor-tabs > div").filter({ has: page.getByRole("button", { name: "main.tex", exact: true }) });
  await expect(main).toHaveClass(/diff-pending/);
  const selectedStyle = await main.evaluate(element => {
    const style = getComputedStyle(element);
    return { background: style.backgroundColor, underline: style.boxShadow, color: style.color };
  });
  await page.locator(".writing-agent-inline-diff-list button").filter({ hasText: "sections/new.tex" }).click();
  const history = page.getByRole("dialog", { name: "副本变更审阅", exact: true });
  await history.getByRole("button", { name: "跳转到编辑器", exact: true }).click();
  const created = pane.locator(".editor-tabs > div").filter({ has: page.getByRole("button", { name: "new.tex", exact: true }) });
  await expect(created).toHaveClass("active");
  await expect(pane.locator(".cm-content")).toContainText("New section");
  expect(await created.evaluate(element => {
    const style = getComputedStyle(element);
    return { background: style.backgroundColor, underline: style.boxShadow, color: style.color };
  })).toEqual(selectedStyle);
  await pane.getByRole("button", { name: "main.tex", exact: true }).click();
  await pane.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(main).toHaveClass("active");
  expect(await main.evaluate(element => {
    const style = getComputedStyle(element);
    return { background: style.backgroundColor, underline: style.boxShadow, color: style.color };
  })).toEqual(selectedStyle);
  await pane.screenshot({ path: ".tmp/writing-source-tab-style.png" });
});

test("withdrawn history remains read-only across preview restart and never replaces the file editor", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/?demo=writing-editor-diff");
  const pane = page.getByRole("region", { name: "编辑器 Diff 预览", exact: true });
  const editor = pane.locator(".cm-content");
  const card = page.locator(".writing-agent-inline-diff-card");
  const expectFrozenStatistics = async (): Promise<void> => {
    await expect(card.locator(".writing-agent-inline-diff-counts")).toHaveText("+9 −3");
    await expect(card.locator(".writing-agent-inline-diff-list small")).toHaveText(["+2 −2", "+2 −1", "+5 −0"]);
  };
  await expectFrozenStatistics();
  await expect(card.locator(".writing-agent-inline-diff-undo")).toBeEnabled();
  await pane.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(pane.locator(".cm-deletedChunk")).toHaveCount(0);
  await expectFrozenStatistics();
  await card.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(card.getByRole("button", { name: "Undo", exact: true })).toBeDisabled();
  await expect(pane.locator(".cm-deletedChunk")).toHaveCount(2);
  await expectFrozenStatistics();
  await expect(pane.locator(".cm-line").filter({ hasText: "The summary appears on the cover page" })).toBeVisible();
  const withdrawn = await editor.innerText();

  for (let pass = 0; pass < 2; pass++) {
    const current = await editor.elementHandle();
    await card.locator(".writing-agent-inline-diff-list button").filter({ hasText: "main.tex" }).click();
    const history = page.getByRole("dialog", { name: "副本变更审阅", exact: true });
    const frozen = history.getByRole("textbox", { name: "逐行差异（只读）", exact: true });
    await expect(frozen).toHaveAttribute("contenteditable", "false");
    await expect(history.locator(".cm-deletedChunk")).toContainText(["The summary appears on the cover page", "\\lipsum[1]"]);
    await expect(history.locator(".cm-line").filter({ hasText: "This paper presents" })).toBeVisible();
    await expect(history.locator(".writing-diff-control-row, .writing-diff-chunk-actions, .cm-merge-revert")).toHaveCount(0);
    const frozenText = await frozen.innerText();
    await frozen.click();
    await page.keyboard.press("Backspace");
    expect(await frozen.innerText()).toBe(frozenText);
    await history.getByRole("button", { name: "关闭 Diff 详情", exact: true }).click();
    expect(await current!.evaluate(element => element.isConnected)).toBe(true);
    expect(await editor.innerText()).toBe(withdrawn);
    await page.getByRole("button", { name: "模拟退出并重开", exact: true }).click();
    await expect.poll(() => current!.evaluate(element => element.isConnected)).toBe(false);
    await expect(editor).toHaveText(withdrawn, { useInnerText: true });
    await expect(pane.locator(".cm-deletedChunk")).toHaveCount(2);
    await expectFrozenStatistics();
  }
  expect(errors).toEqual([]);
});
