import { expect, test } from "@playwright/test";

for (const entry of ["/?preview=app", "/?demo=app"]) test(`library and PDF tabs share the window toolbar without rebuilding the reader at ${entry}`, async ({ page }) => {
  await page.goto(entry);
  const toolbar = page.getByRole("banner", { name: "窗口工具栏", exact: true });
  const tabs = toolbar.getByRole("tablist", { name: "打开的标签页", exact: true });
  const workspace = page.locator(".library-workspace");
  await expect(tabs).toBeVisible();
  await expect(tabs.getByRole("tab", { name: "文献", exact: true })).toHaveCount(0);
  await expect(workspace.getByRole("tablist")).toHaveCount(0);
  const header = await toolbar.boundingBox();
  const tabBounds = await tabs.boundingBox();
  expect(header!.height).toBe(36);
  expect(tabBounds!.y).toBe(header!.y);
  expect(tabBounds!.height).toBe(header!.height);
  expect((await workspace.boundingBox())!.y).toBe(header!.y + header!.height);
  const home = tabs.getByRole("tab", { name: "返回文献列表", exact: true });
  const homeFrame = home.locator("..");
  await expect(home).toHaveAttribute("aria-selected", "true");
  await expect(homeFrame).toHaveCSS("background-color", "rgb(255, 255, 255)");
  const homeBounds = (await homeFrame.boundingBox())!;
  expect(Math.abs(homeBounds.x - tabBounds!.x - 1)).toBeLessThan(1);

  const title = "Synthetic Research Preview: Evidence and Autonomy";
  await page.getByRole("row", { name: new RegExp(title) }).dblclick();
  const reader = page.getByLabel(`${title} PDF 阅读器`, { exact: true });
  await expect(reader).toBeVisible();
  // Wait for PDF readiness so the loading status cannot shift the hover target.
  await expect(reader.locator(".pdf-view .textLayer span").first()).toBeVisible();
  const readerHomeBounds = (await homeFrame.boundingBox())!;
  expect(readerHomeBounds.x).toBe(homeBounds.x);
  const activeCard = tabs.locator('.workspace-tab[data-active="true"]:not([data-home="true"])');
  await expect(activeCard).toHaveCSS("border-radius", "8px");
  await expect(activeCard).toHaveCSS("background-color", "rgb(255, 255, 255)");
  const activeFrame = await activeCard.evaluate(element => ({border: getComputedStyle(element).borderColor, shadow: getComputedStyle(element).boxShadow}));
  await expect(home).toHaveAttribute("aria-selected", "false");
  await expect(homeFrame).toHaveCSS("box-shadow", "none");
  const cardBounds = await activeCard.boundingBox();
  expect(cardBounds!.height).toBe(26);
  expect(cardBounds!.y - header!.y).toBe(5);
  await page.screenshot({ path: ".tmp/window-rounded-tabs.png" });
  await expect(reader.getByRole("button", { name: "关闭阅读器", exact: true })).toHaveCount(0);
  const structure = reader.getByRole("button", { name: "显示 Docling 结构化块", exact: true });
  await expect(structure).toHaveText("");
  await expect(structure).toHaveAttribute("aria-pressed", "false");
  await expect(structure).toHaveCSS("width", "28px");
  await structure.hover();
  await expect(page.getByRole("tooltip", { name: "显示结构化块", exact: true })).toBeVisible();
  const readerInstance = await reader.elementHandle();
  await tabs.getByRole("tab", { name: "返回文献列表", exact: true }).click();
  await expect(reader).toBeHidden();
  await expect(homeFrame).toHaveCSS("background-color", "rgb(255, 255, 255)");
  await expect(homeFrame).toHaveCSS("border-color", activeFrame.border);
  await expect(homeFrame).toHaveCSS("box-shadow", activeFrame.shadow);
  await page.screenshot({path: `.tmp/library-home-alignment-${entry.includes("demo") ? "demo" : "preview"}.png`});
  await expect(activeCard).toHaveCount(0);
  await expect(tabs.locator('.workspace-tab:not([data-home="true"])')).toHaveCSS("box-shadow", "none");
  await tabs.getByRole("tab", { name: title, exact: true }).click();
  await expect(reader).toBeVisible();
  expect(await readerInstance!.evaluate(element => element.isConnected)).toBe(true);
  await tabs.getByRole("button", { name: `关闭 ${title}`, exact: true }).click();
  await expect(page.locator(".library-surface")).toBeVisible();
  await expect(tabs.getByRole("tab")).toHaveCount(1);
  await expect(reader).toHaveCount(0);

  await page.getByRole("button", { name: "设置", exact: true }).click();
  await expect(tabs).toBeVisible();
  await expect(home).toHaveAttribute("aria-selected", "false");
  await page.getByRole("button", { name: "后退", exact: true }).click();
  await expect(tabs).toBeVisible();
});

test("navigation divider follows the wider sidebar or complete icon group", async ({ page }) => {
  await page.goto("/?preview=app");
  const toolbar = page.getByRole("banner", { name: "窗口工具栏", exact: true });
  const navigation = toolbar.getByRole("toolbar", { name: "工作区导航", exact: true });
  const sidebar = page.getByRole("complementary", { name: "主导航", exact: true });
  const bounds = await navigation.boundingBox();
  const expanded = await sidebar.boundingBox();
  expect(bounds!.x + bounds!.width).toBeCloseTo(expanded!.x + expanded!.width, 1);
  expect(await navigation.evaluate(element => getComputedStyle(element, "::after").width)).toBe("1px");

  await toolbar.getByRole("button", { name: "收起主导航", exact: true }).click();
  const collapsed = await sidebar.boundingBox();
  const iconGroup = await navigation.boundingBox();
  expect(iconGroup!.x + iconGroup!.width).toBeGreaterThan(collapsed!.x + collapsed!.width);
  expect(iconGroup!.width).toBeLessThan(bounds!.width);

  // Native controls occupy 96px even when the collapsed sidebar is only 64px.
  await toolbar.evaluate(element => element.setAttribute("data-native-controls", "true"));
  const nativeGroup = await navigation.boundingBox();
  expect(nativeGroup!.x).toBe(96);
  expect(nativeGroup!.x + nativeGroup!.width).toBeGreaterThan(iconGroup!.x + iconGroup!.width);
  await toolbar.getByRole("button", { name: "展开主导航", exact: true }).click();
  const nativeExpanded = await navigation.boundingBox();
  expect(nativeExpanded!.x + nativeExpanded!.width).toBeCloseTo(expanded!.width, 1);
  await toolbar.evaluate(element => element.removeAttribute("data-native-controls"));

  await page.setViewportSize({ width: 640, height: 808 });
  await expect(toolbar.getByRole("button", { name: "收起主导航", exact: true })).toBeHidden();
  const narrow = await navigation.boundingBox();
  expect(narrow!.width).toBeLessThan(iconGroup!.width);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
});

test("window tabs scroll within their available width on a narrow screen", async ({ page }) => {
  await page.setViewportSize({ width: 640, height: 808 });
  await page.goto("/?preview=app");
  const tabs = page.getByRole("tablist", { name: "打开的标签页", exact: true });
  for (const title of ["Vision by Logic", "Synthetic Research Preview", "Life-inspired interoceptive"]) {
    const row = page.getByRole("row", {name: new RegExp(title)});
    const label = row.locator("strong");
    const fullTitle = (await label.textContent())!;
    await label.click();
    await expect(page.locator(".literature-detail").getByRole("heading", {name: fullTitle, exact: true})).toBeVisible();
    await label.dblclick();
    await expect(tabs.getByRole("tab", {name: new RegExp(title)})).toHaveAttribute("aria-selected", "true");
    await tabs.getByRole("tab", { name: "返回文献列表", exact: true }).click();
  }
  expect(await tabs.evaluate(element => element.scrollWidth > element.clientWidth)).toBe(true);
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= window.innerWidth)).toBe(true);
  await tabs.getByRole("tab", { name: /Life-inspired interoceptive/ }).click();
  await expect(tabs.getByRole("tab", { name: /Life-inspired interoceptive/ })).toHaveAttribute("aria-selected", "true");
  await tabs.getByRole("button", { name: /关闭 Life-inspired interoceptive/ }).click();
  await expect(tabs.getByRole("tab", { name: /Life-inspired interoceptive/ })).toHaveCount(0);
});

test("writing file tabs dock in the window toolbar and preserve editing across workspaces", async ({ page }) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await page.getByRole("button", { name: "Autonomy（合成稿件） main.tex", exact: true }).click();
  const toolbar = page.getByRole("banner", { name: "窗口工具栏", exact: true });
  const tabs = toolbar.getByRole("tablist", { name: "打开的标签页", exact: true });
  const editor = page.locator(".writing-editor-host .cm-content");
  await expect(editor).toBeVisible();
  // Settle the preview's seeded Agent changes before testing user edits.
  await page.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(page.locator(".cm-deletedChunk")).toHaveCount(0);
  await expect(tabs).toBeVisible();
  await expect(page.locator(".writing-editor-column > .editor-tabs")).toHaveCount(0);
  const header = await toolbar.boundingBox();
  const active = tabs.locator('.workspace-tab[data-active="true"]:not([data-home="true"])');
  await expect(active).toHaveCSS("border-radius", "8px");
  expect((await active.boundingBox())!.y - header!.y).toBe(5);
  expect((await active.boundingBox())!.height).toBe(26);
  const view = await page.locator(".cm-editor").elementHandle();
  await editor.click(); await page.keyboard.press("Control+Home");
  await page.keyboard.insertText("% Window tabs preserve input\n");
  await expect(page.locator(".writing-toolbar .save-state")).toHaveText("已保存");
  await page.locator(".writing-file-tree").getByRole("button", { name: "overview.tex", exact: true }).click();
  await expect(tabs.getByRole("tab", { name: /^overview\.tex(?: |$)/ })).toBeVisible();
  await expect(editor).toContainText("Overview of the proposed framework");
  await tabs.getByRole("tab", { name: "main.tex", exact: true }).click();
  await expect(editor).toContainText("Window tabs preserve input");
  expect(await view!.evaluate(node => node.isConnected)).toBe(true);
  await page.screenshot({ path: ".tmp/window-writing-tabs.png" });
  await page.getByRole("button", { name: "文献", exact: true }).click();
  await expect(tabs).toBeVisible();
  await expect(tabs.getByRole("tab", {name: "main.tex", exact: true})).toHaveAttribute("aria-selected", "false");
  await expect(toolbar.getByRole("tablist", { name: "打开的标签页", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "设置", exact: true }).click();
  await expect(tabs).toBeVisible();
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await expect(tabs).toBeVisible(); await expect(editor).toContainText("Window tabs preserve input");
  expect(await view!.evaluate(node => node.isConnected)).toBe(true);
  await tabs.getByRole("button", { name: "关闭 overview.tex", exact: true }).click();
  await expect(tabs.getByRole("tab", { name: /^overview\.tex(?: |$)/ })).toHaveCount(0);
  await page.setViewportSize({ width: 640, height: 808 });
  await expect(tabs).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  await page.getByRole("tab", { name: "返回写作项目列表", exact: true }).click();
  await expect(tabs).toBeVisible();
  await expect(tabs.getByRole("tab", { name: "返回写作项目列表", exact: true })).toHaveAttribute("aria-selected", "true");
  await expect(tabs.locator('.workspace-tab[data-active="true"]:not([data-home="true"])')).toHaveCount(0);
  await page.getByRole("button", { name: "Autonomy（合成稿件） main.tex", exact: true }).click();
  await expect(tabs).toBeVisible(); await expect(editor).toContainText("Window tabs preserve input");
});
