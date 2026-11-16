import {expect, test} from "@playwright/test";
import {holdLiveAgentOutput, openLiveAgent} from "./live-agent";

test("a collapsed running chat animates its launcher and remains keyboard accessible", async ({page}) => {
  const input = await openLiveAgent(page);
  await holdLiveAgentOutput(page);
  await input.fill("检查运行状态");
  await page.getByRole("button", {name: "发送消息", exact: true}).click();
  await expect(page.getByRole("button", {name: "停止生成", exact: true})).toBeVisible();
  await page.getByRole("button", {name: "隐藏对话", exact: true}).click();
  const launcher = page.getByRole("button", {name: "打开对话", exact: true});
  const spinner = launcher.locator(".agent-launcher-spinner");
  await expect(launcher).toHaveAttribute("aria-busy", "true");
  await expect(spinner).toBeVisible();
  await expect(spinner).toHaveCSS("animation-name", "agent-launcher-spin");
  await expect(spinner).toHaveCSS("animation-duration", "2s");
  await expect(spinner.locator(".agent-spinner-track")).toHaveCSS("stroke-opacity", "0.25");
  await expect(spinner.locator(".agent-spinner-arc")).toHaveCSS("stroke-opacity", "1");
  await expect(spinner.locator(".agent-spinner-arc")).toHaveAttribute("stroke-dasharray", "75 25");
  await launcher.screenshot({path: ".tmp/agent-launcher-running.png"});
  await page.emulateMedia({reducedMotion: "reduce"});
  await expect(spinner).toHaveCSS("animation-name", "none");
  await expect(launcher).toHaveAttribute("aria-busy", "true");
  await launcher.focus();
  await page.keyboard.press("Enter");
  await expect(page.getByRole("button", {name: "停止生成", exact: true})).toBeVisible();
  await page.getByRole("button", {name: "停止生成", exact: true}).click();
  await expect(page.locator('[data-agent-activity="aborted"]').last()).toBeVisible();
  await page.getByRole("button", {name: "隐藏对话", exact: true}).click();
  await expect(launcher).toHaveAttribute("aria-busy", "false");
  await expect(spinner).toHaveCount(0);
  await expect(launcher.locator(".lucide-message-circle")).toBeVisible();
});

test("sending preparation stays busy while hidden and a preparation failure restores the icon and draft", async ({page}) => {
  const input = await openLiveAgent(page);
  await page.evaluate(async () => {
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).holdLauncherPreparation();
  });
  await input.fill("检查转圈状态");
  await page.getByRole("button", {name: "发送消息", exact: true}).click();
  await expect(page.getByRole("status", {name: "输入框准备状态", exact: true})).toBeVisible();
  await expect.poll(() => page.evaluate(async () =>
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).launcherPreparationReached())).toBe(true);
  await page.getByRole("button", {name: "隐藏对话", exact: true}).click();
  const launcher = page.getByRole("button", {name: "打开对话", exact: true});
  await expect(launcher).toHaveAttribute("aria-busy", "true");
  await page.evaluate(async () => {
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).failHeldLauncherPreparation();
  });
  await expect(launcher).toHaveAttribute("aria-busy", "false");
  await expect(launcher.locator(".agent-launcher-spinner")).toHaveCount(0);
  await launcher.click();
  await expect(input).toHaveText("检查转圈状态");
});

test("new chat sends clear the owned draft and compact running rows reveal actions on hover or focus", async ({page}) => {
  await page.setViewportSize({width: 1272, height: 808});
  const input = await openLiveAgent(page);
  await holdLiveAgentOutput(page);
  await input.fill("后台任务");
  await page.getByRole("button", {name: "发送消息", exact: true}).click();
  await expect(page.getByRole("button", {name: "停止生成", exact: true})).toBeVisible();
  await page.getByRole("button", {name: "新建会话", exact: true}).click();
  await input.fill("hi");
  await page.getByRole("button", {name: "发送消息", exact: true}).click();
  await expect(page.getByRole("button", {name: "停止生成", exact: true})).toBeVisible();
  await expect(input).toBeEmpty();
  const toggle = page.getByRole("button", {name: "对话列表", exact: true});
  if (await toggle.getAttribute("aria-expanded") === "false") await toggle.click();
  const row = page.locator(".conversation-list-row.active");
  const indicator = row.getByRole("status", {name: "对话正在运行", exact: true});
  await page.mouse.move(0, 0);
  await expect(row).toHaveCSS("height", "30px");
  await expect(indicator).toHaveCSS("opacity", "1");
  await expect(row.locator(".conversation-row-actions")).toHaveCSS("opacity", "0");
  await page.locator(".agent-canvas-conversations").screenshot({path: ".tmp/agent-conversation-running.png"});
  await row.hover();
  await expect(indicator).toHaveCSS("opacity", "0");
  await expect(row.locator(".conversation-row-actions")).toHaveCSS("opacity", "1");
  await page.mouse.move(0, 0);
  await row.getByRole("button").first().focus();
  await expect(indicator).toHaveCSS("opacity", "0");
  await expect(row.locator(".conversation-row-actions")).toHaveCSS("opacity", "1");
  await page.getByRole("button", {name: "停止生成", exact: true}).click();
  await expect(page.getByRole("button", {name: "停止生成", exact: true})).toHaveCount(0);
  // The browser adapter has no native activity subscription. Deliver the same
  // invalidation after its real interrupt has settled.
  await page.evaluate(async () => {
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).refreshLauncherActivity();
  });
  await expect(indicator).toHaveCount(0);
  await page.setViewportSize({width: 685, height: 808});
  await expect(row).toHaveCSS("height", "30px");
});

test("internal activity retains the robot inside a centered ring and stops when activity ends", async ({page}) => {
  await openLiveAgent(page);
  await page.evaluate(async () => {
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).setLauncherInternalActivity(true);
  });
  const toggle = page.getByRole("button", {name: "对话列表", exact: true});
  if (await toggle.getAttribute("aria-expanded") === "false") await toggle.click();
  const internal = page.getByRole("button", {name: "内部对话", exact: true});
  await expect(internal).toHaveAttribute("aria-busy", "true");
  const bot = internal.locator(".lucide-bot");
  const ring = internal.locator(".agent-launcher-spinner");
  await expect(bot).toBeVisible();
  await expect(ring).toBeVisible();
  await expect(ring.locator(".agent-spinner-track")).toHaveCSS("stroke-opacity", "0.25");
  await expect(ring.locator(".agent-spinner-arc")).toHaveCSS("stroke-opacity", "1");
  await expect(ring).toHaveCSS("width", "23px");
  await expect(ring).toHaveCSS("height", "23px");
  await expect(ring).toHaveAttribute("stroke-width", "1.5");
  const [iconBox, ringBox] = await Promise.all([bot.boundingBox(), ring.boundingBox()]);
  expect(Math.abs(iconBox!.x + iconBox!.width / 2 - ringBox!.x - ringBox!.width / 2)).toBeLessThan(1);
  expect(Math.abs(iconBox!.y + iconBox!.height / 2 - ringBox!.y - ringBox!.height / 2)).toBeLessThan(1);
  await internal.screenshot({path: ".tmp/agent-internal-running.png"});
  await internal.click();
  await expect(page.getByRole("complementary", {name: "对话列表面板", exact: true})).toContainText("暂无内部对话");
  await page.getByRole("button", {name: "返回普通对话", exact: true}).click();
  await page.emulateMedia({reducedMotion: "reduce"});
  await expect(ring).toHaveCSS("animation-name", "none");
  await page.evaluate(async () => {
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).setLauncherInternalActivity(false);
  });
  await expect(internal).toHaveAttribute("aria-busy", "false");
  await expect(ring).toHaveCount(0);
  await expect(bot).toBeVisible();
});
