import { expect, test, type Locator, type Page } from "@playwright/test";
import { editorOverlay } from "./editorOverlay";

async function expectBlockControls(page: Page, surface: Locator): Promise<void> {
  await surface.locator(".bn-editor p").first().hover({ position: { x: 8, y: 8 } });
  const overlay = await editorOverlay(page, surface);
  const add = overlay.getByRole("button", { name: "添加块", exact: true });
  const drag = overlay.getByRole("button", { name: "拖动块；点击打开块菜单", exact: true });
  await expect(add).toHaveCSS("width", "24px");
  await expect(add).toHaveCSS("height", "32px");
  await expect(drag).toHaveCSS("width", "18px");
  await expect(drag).toHaveCSS("height", "32px");
  await expect(drag).toHaveCSS("margin-right", "5px");
  await expect(overlay.locator(".bn-side-menu")).toHaveCSS("padding-right", "8px");
  await expect(surface.locator(".bn-editor p").first()).toHaveCSS("font-size", "16px");
  await expect(surface.locator(".bn-block-outer").first()).toHaveCSS("padding-left", "8px");
}

async function expectWritingLeftMarquee(page: Page, surface: Locator): Promise<void> {
  const pane = page.locator(".writing-note-session:not([hidden]) .writing-note-scroll");
  await expect(pane).toHaveCSS("padding-left", "24px");
  const paneBox = await pane.boundingBox(), surfaceBox = await surface.boundingBox();
  const rows = surface.locator(".bn-block-content");
  const first = await rows.nth(0).boundingBox(), second = await rows.nth(1).boundingBox();
  if (!paneBox || !surfaceBox || !first || !second) throw new Error("Missing writing marquee geometry");
  const start = { x: paneBox.x + 12, y: first.y + 1 };
  expect(start.x).toBeLessThan(surfaceBox.x);
  await page.mouse.move(start.x, start.y); await page.mouse.down();
  await page.mouse.move(second.x + Math.min(100, second.width - 2), second.y + second.height - 1, { steps: 12 });
  await expect(page.locator(".note-block-selection-rect")).toBeVisible();
  await expect(surface.locator(".bn-block-outer").nth(0)).toHaveCSS("background-color", "rgb(234, 242, 237)");
  await expect(surface.locator(".bn-block-outer").nth(1)).toHaveCSS("background-color", "rgb(234, 242, 237)");
  await page.screenshot({ path: test.info().outputPath("writing-left-marquee-" + page.viewportSize()!.width + ".png") });
  await page.mouse.up();
  await expect(page.locator(".note-block-selection-rect")).toHaveCount(0);
  await page.keyboard.press("Escape");
}

for (const url of ["/", "/?preview=app"]) {
  test(`notes and manuscript notes share typography and block controls on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
    const title = page.getByRole("textbox", { name: "笔记标题", exact: true });
    await title.fill("共享样式");
    const note = page.getByRole("group", { name: "笔记正文", exact: true });
    await note.locator(".bn-editor").click();
    await page.keyboard.insertText("shared body");
    await expect(title).toHaveCSS("font-size", "40px");
    await expect(title).toHaveCSS("line-height", "46.4px");
    await expectBlockControls(page, note);

    await page.getByRole("button", { name: "写作", exact: true }).click();
    await page.getByRole("button", { name: "新建稿件", exact: true }).click();
    await page.getByRole("dialog", { name: "新建 LaTeX 稿件" }).getByRole("textbox").fill("Shared note styles");
    await page.getByRole("button", { name: "创建稿件", exact: true }).click();
    await page.getByRole("tab", { name: "笔记", exact: true }).click();
    await page.getByRole("button", { name: "新建笔记", exact: true }).click();
    const name = page.getByRole("textbox", { name: "笔记名称", exact: true });
    await name.fill("共享样式"); await name.press("Enter");
    const writing = page.getByRole("group", { name: "写作笔记正文", exact: true });
    const heading = writing.locator("h1");
    await expect(heading).toHaveCSS("font-size", "40px");
    await expect(heading).toHaveCSS("line-height", "46.4px");
    await heading.fill("共享样式"); await page.keyboard.press("Enter");
    await page.keyboard.insertText("shared body");
    await expect(heading).toHaveText("共享样式");
    await expectWritingLeftMarquee(page, writing);
    await expectBlockControls(page, writing);
    await (await editorOverlay(page, writing)).getByRole("button", { name: "拖动块；点击打开块菜单", exact: true }).hover();
    await page.screenshot({ path: test.info().outputPath("writing-note-shared-styles.png") });
    await page.setViewportSize({ width: 900, height: 720 });
    await expectBlockControls(page, writing);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
    await page.setViewportSize({ width: 680, height: 720 });
    await expect(heading).toHaveCSS("font-size", "34px");
    await expectBlockControls(page, writing);
  });
}
