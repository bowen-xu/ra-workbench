import { expect, test } from "@playwright/test";
import { PREVIEW_STORAGE_PREFIX } from "../../apps/web/src/core/runtime/appStorage";

test("new conversations acquire a title and long titles yield space to header controls", async ({ page }) => {
  await page.goto("/?preview=app");
  await page.getByRole("row", { name: /Synthetic Research Preview/ }).dblclick();
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  const panel = page.locator(".writing-agent-panel:visible");
  const title = panel.locator(".writing-agent-header-subtitle");
  const input = page.getByRole("textbox", { name: "给研究助手发送消息", exact: true });
  await expect(title).toHaveText("新对话");
  await expect(panel.getByRole("button", { name: /^移除推荐上下文 Synthetic Research Preview/ })).toBeVisible();
  await input.fill("请帮我分析这篇论文的研究方法");
  await page.getByRole("button", { name: "发送消息", exact: true }).click();
  await expect(title).toHaveText("分析这篇论文的研究方法");
  await expect(page.getByRole("button", { name: "停止生成", exact: true })).toHaveCount(0);
  const longTitle = "论文分析 · 4e25e945-f618-4d49-b82b-7d3fe07 长对话名称需要优先压缩并保留操作入口";
  await title.dblclick();
  await panel.getByRole("textbox", { name: "对话名称", exact: true }).fill(longTitle);
  await page.keyboard.press("Tab");
  await expect(title).toHaveText(longTitle);
  const leftEdge = page.getByRole("separator", { name: "调整研究助手左边界", exact: true });
  await leftEdge.focus();
  for (let step = 0; step < 4; step++) await leftEdge.press("ArrowLeft");
  for (const width of [1276, 685, 420]) {
    await page.setViewportSize({ width, height: 808 });
    const metrics = () => panel.locator(".writing-agent-header").evaluate(header => {
      const controls = header.querySelector(".agent-context-header-controls")!.getBoundingClientRect();
      const titleNode = header.querySelector(".writing-agent-header-subtitle")!;
      const text = titleNode.getBoundingClientRect();
      const dock = header.querySelector(".writing-agent-dock-handle")!.getBoundingClientRect();
      const bounds = header.getBoundingClientRect();
      return { overlap: text.right - controls.left, overflow: dock.right - bounds.right,
        clipped: titleNode.scrollWidth > titleNode.clientWidth };
    });
    await expect.poll(async () => (await metrics()).overflow).toBeLessThanOrEqual(0);
    const result = await metrics();
    expect(result.overlap).toBeLessThanOrEqual(0);
    expect(result.clipped).toBe(true);
    await expect(panel.locator(".agent-context-recommendation-row")).toHaveCount(0);
    await expect(panel.getByRole("button", { name: "上下文管理", exact: true })).toBeInViewport();
    await panel.locator(".writing-agent-header").screenshot({ path: `.tmp/chat-long-title-${width}.png` });
  }
  await page.setViewportSize({ width: 1276, height: 808 });
  await panel.getByRole("button", { name: "对话列表", exact: true }).click();
  await expect(page.getByRole("complementary", { name: "对话列表面板" }).getByRole("button", { name: longTitle, exact: true })).toBeVisible();
});

test("switching back to a streaming conversation shows its received prefix immediately", async ({ page }) => {
  await page.addInitScript(prefix => localStorage.setItem(prefix + "controls", JSON.stringify({ delayMs: 0, nextFailure: null })), PREVIEW_STORAGE_PREFIX);
  await page.clock.install({ time: new Date("2026-10-16T12:00:00Z") });
  await page.goto("/?preview=app");
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  const panel = page.locator(".writing-agent-panel:visible");
  const title = panel.locator(".writing-agent-header-subtitle");
  await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill("切换对话动效验收");
  await page.clock.pauseAt(new Date(await page.evaluate(() => Date.now() + 100)));
  // Keep provider output paused during interaction; actionability was verified
  // before pausing, so clicks need not wait for frozen animation frames.
  await page.getByRole("button", { name: "发送消息", exact: true }).click({ force: true });
  await page.clock.runFor(100);
  await expect(title).toHaveText("切换对话动效验收");
  await page.clock.runFor(100);
  const markdown = panel.locator(".writing-agent-markdown").last();
  await expect(markdown).not.toHaveText("");
  const prefix = await markdown.textContent();
  await panel.getByRole("button", { name: "新建会话", exact: true }).click({ force: true });
  await expect(title).toHaveText("新对话");
  await panel.getByRole("button", { name: "对话列表", exact: true }).click({ force: true });
  await page.getByRole("complementary", { name: "对话列表面板" }).getByRole("button", { name: "切换对话动效验收", exact: true }).click({ force: true });
  await expect(title).toHaveText("切换对话动效验收");
  await expect(markdown).toHaveText(prefix!);
  const restored = await markdown.evaluate(element => ({
    opacity: getComputedStyle(element).opacity, animations: element.getAnimations().length,
    progress: [...element.querySelectorAll<HTMLElement>(".stream-reveal-glyph")].map(node => node.style.getPropertyValue("--stream-reveal-progress")),
  }));
  expect(restored.opacity).toBe("1");
  expect(restored.animations).toBe(0);
  expect(restored.progress.every(value => value === "1")).toBe(true);
  await page.clock.runFor(60);
  expect((await markdown.textContent())!.length).toBeGreaterThan(prefix!.length);
  expect(await markdown.locator(".stream-reveal-glyph").last().evaluate(node => Number((node as HTMLElement).style.getPropertyValue("--stream-reveal-progress")))).toBeLessThan(1);
});
