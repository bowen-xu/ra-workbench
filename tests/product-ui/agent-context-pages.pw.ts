import {expect, test, type Page} from "@playwright/test";
import {openLiveAgent, holdLiveAgentOutput} from "./live-agent";

async function finishPageAnimation(page: Page): Promise<void> {
  await expect.poll(() => page.locator(".context-catalog-viewport .context-catalog-page").evaluate(el => el.getAnimations().every(animation => animation.playState === "finished"))).toBe(true);
  await expect(page.locator(".context-catalog-outgoing")).toHaveCount(0);
}

test("the production App running context opens with mock data and replaces pages on deliberate scrolling", async ({page}) => {
  await page.setViewportSize({width: 1035, height: 668});
  const input = await openLiveAgent(page);
  await holdLiveAgentOutput(page);
  await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).installLiveContextCatalog());
  await input.fill("检查运行中上下文分页");
  await page.getByRole("button", {name: "发送消息", exact: true}).click();
  await expect(page.getByRole("button", {name: "停止生成", exact: true})).toBeVisible();
  const trigger = page.getByRole("button", {name: "上下文管理", exact: true});
  await expect(trigger).toBeEnabled(); await trigger.click();
  const dialog = page.getByRole("dialog", {name: "上下文管理", exact: true});
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("status")).toHaveText("第 1 / 5 页 · 37 项");
  await expect(dialog.locator(".context-catalog-viewport .context-catalog > li")).toHaveCount(7);
  await expect(dialog.locator("footer")).toHaveCount(0);
  await expect(dialog.getByRole("button", {name: "上一页上下文", exact: true})).toHaveCount(0);
  await expect(dialog.getByLabel("使用 模拟文献 01 · 研究方法与可复现性", {exact: true})).toBeDisabled();
  const viewport = dialog.getByRole("region", {name: "上下文分页内容", exact: true});
  await viewport.hover(); await page.mouse.wheel(0, 180);
  await expect(dialog.locator("[data-page]")).toHaveAttribute("data-page", "2");
  await expect(dialog.locator(".context-catalog-viewport .context-catalog-page")).toHaveCSS("animation-name", "context-page-next");
  await finishPageAnimation(page);
  await expect(dialog.getByLabel("使用 模拟文献 01 · 研究方法与可复现性", {exact: true})).toHaveCount(0);
  await page.screenshot({path: ".tmp/context-pages-canvas.png"});
  await viewport.hover(); await page.mouse.wheel(0, -180);
  await expect(dialog.locator("[data-page]")).toHaveAttribute("data-page", "1");
  await finishPageAnimation(page);
  await dialog.getByRole("textbox", {name: "搜索上下文", exact: true}).fill("模拟笔记");
  await expect(dialog.getByRole("status")).toContainText("第 1 / 2 页");
  await expect(dialog.getByLabel("使用 模拟笔记 01 · 研究方法与可复现性", {exact: true})).toBeVisible();
  await expect(page.getByText("无法核对写作项目权限，请重试。", {exact: true})).toHaveCount(0);
  await page.keyboard.press("Escape"); await expect(dialog).not.toBeVisible(); await expect(trigger).toBeFocused();
});

test("the writing Diff preview keeps edited context selections across page replacement", async ({page}) => {
  await page.goto("/?demo=writing-editor-diff");
  await page.getByRole("button", {name: "上下文管理", exact: true}).click();
  const dialog = page.getByRole("dialog", {name: "上下文管理", exact: true});
  const label = "使用 模拟文献 01 · 研究方法与可复现性";
  await dialog.getByLabel(label, {exact: true}).check();
  await expect(dialog.getByLabel(label, {exact: true})).toBeChecked();
  await dialog.getByRole("button", {name: "下一页上下文", exact: true}).click();
  await expect(dialog.locator("[data-page]")).toHaveAttribute("data-page", "2");
  await finishPageAnimation(page);
  await dialog.getByRole("button", {name: "上一页上下文", exact: true}).click();
  await finishPageAnimation(page);
  await expect(dialog.getByLabel(label, {exact: true})).toBeChecked();
  await dialog.getByRole("button", {name: "下一页上下文", exact: true}).hover();
  const hint = page.getByRole("tooltip").filter({hasText: "下一页，也可向下滚动"});
  await expect(hint).toBeVisible();
  expect(await hint.evaluate(el => Boolean(el.closest("dialog[open]")))).toBe(true);
  const next = dialog.getByRole("button", {name: "下一页上下文", exact: true});
  expect(await next.evaluate(el => {const box = el.getBoundingClientRect(); return el.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2));})).toBe(true);
  await page.screenshot({path: ".tmp/context-pages-writing-hint.png"});
});

test("the full app preview uses the same context pages within a narrow native dialog", async ({page}) => {
  await page.setViewportSize({width: 600, height: 650});
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  const trigger = page.getByRole("button", {name: "上下文管理", exact: true});
  await trigger.click();
  const dialog = page.getByRole("dialog", {name: "上下文管理", exact: true});
  await expect(dialog.locator("footer")).toHaveCount(0);
  await expect(dialog.locator(".context-catalog-page-edge").last().getByRole("button", {name: "下一页上下文", exact: true})).toBeVisible();
  await expect(dialog.locator(".context-catalog-item").first()).toBeVisible();
  const bounds = await dialog.boundingBox();
  expect(bounds!.x).toBeGreaterThanOrEqual(0); expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(600);
  expect(bounds!.y).toBeGreaterThanOrEqual(0); expect(bounds!.y + bounds!.height).toBeLessThanOrEqual(650);
  await page.screenshot({path: ".tmp/context-pages-app-narrow.png"});
  await dialog.getByRole("textbox", {name: "搜索上下文", exact: true}).fill("无匹配上下文fixture-absent");
  await expect(dialog.getByText("没有匹配的上下文对象", {exact: true})).toBeVisible();
  await expect(dialog.getByRole("button", {name: "下一页上下文", exact: true})).toHaveCount(0);
  await page.keyboard.press("Escape"); await expect(trigger).toBeFocused();
});

test("reduced motion disables page animation and keyboard paging remains available", async ({page}) => {
  await page.emulateMedia({reducedMotion: "reduce"});
  await page.goto("/?demo=writing-editor-diff");
  await page.getByRole("button", {name: "上下文管理", exact: true}).click();
  const viewport = page.getByRole("region", {name: "上下文分页内容", exact: true});
  await viewport.focus(); await page.keyboard.press("PageDown");
  await expect(page.locator("[data-page]")).toHaveAttribute("data-page", "2");
  await expect(page.locator(".context-catalog-page")).toHaveCSS("animation-name", "none");
});
