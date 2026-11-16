import { expect, test } from "@playwright/test";
import {holdLiveAgentOutput, openLiveAgent, sendLiveAgentMessage} from "./live-agent";

for (const width of [632, 1192]) {
  test(`return-to-bottom visibility never changes the scroll range at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 808 });
    await openLiveAgent(page, {text: Array.from({length: 32}, (_, index) => `### 合成段落 ${index + 1}\n\n验证共享滚动布局与消息输出。\n\n`).join("")});
    await sendLiveAgentMessage(page, "生成滚动验收样例", 1);
    await page.clock.install();
    const scroller = page.locator(".writing-agent-messages");
    const bottom = page.locator(".writing-agent-panel").getByRole("button", { name: "回到底部", exact: true });
    // Measure both states while the transient scrollbar is visible, so a
    // scrollbar-induced text reflow cannot mask the button's layout effect.
    await scroller.evaluate(element => { element.scrollTop = element.scrollHeight - element.clientHeight - 1; });
    await page.clock.runFor(100);
    const height = await scroller.evaluate(element => element.scrollHeight);
    await scroller.evaluate(element => { element.scrollTop -= 120; });
    await page.clock.runFor(100);
    await expect(bottom).toBeVisible();
    expect(await scroller.evaluate(element => element.scrollHeight)).toBe(height);
    const listBounds = (await scroller.boundingBox())!;
    const buttonBounds = (await bottom.boundingBox())!;
    expect(buttonBounds.y).toBeGreaterThanOrEqual(listBounds.y);
    expect(buttonBounds.y + buttonBounds.height).toBeLessThanOrEqual(listBounds.y + listBounds.height);
    const clientWidth = await scroller.evaluate(element => element.clientWidth);
    expect(buttonBounds.x + buttonBounds.width / 2).toBeCloseTo(listBounds.x + clientWidth / 2, 0);
    await scroller.hover();
    await page.mouse.wheel(0, 500);
    await page.clock.runFor(200);
    await expect(bottom).toHaveCount(0);
    const reached = await scroller.evaluate(element => ({ top: element.scrollTop, height: element.scrollHeight,
      distance: element.scrollHeight - element.clientHeight - element.scrollTop }));
    expect(reached.height).toBe(height);
    expect(reached.distance).toBeLessThanOrEqual(6);
    await page.mouse.wheel(0, 100);
    await page.clock.runFor(100);
    expect(await scroller.evaluate(element => element.scrollTop)).toBe(reached.top);
    await scroller.evaluate(element => { element.scrollTop -= 120; });
    await page.clock.runFor(100);
    await bottom.click();
    await page.clock.runFor(100);
    await expect(bottom).toHaveCount(0);
    expect(await scroller.evaluate(element => element.scrollHeight)).toBe(height);
    await page.locator(".writing-agent-panel").screenshot({ path: `.tmp/agent-scroll-bottom-${width}.png` });
    await expect(bottom.locator(".writing-agent-scroll-bottom-dots")).toHaveCount(0);
    await holdLiveAgentOutput(page);
    await page.getByRole("textbox", { name: "给研究助手发送消息" }).fill("检查运行时回到底部图标");
    await page.getByRole("button", { name: "发送消息", exact: true }).click();
    await expect(page.getByRole("button", { name: "停止生成", exact: true })).toBeVisible();
    await page.clock.runFor(50);
    await scroller.evaluate(element => {
      element.scrollTop -= 120;
      element.dispatchEvent(new Event("scroll"));
    });
    await page.clock.runFor(100);
    await page.mouse.move(0, 0);
    const dots = bottom.locator(".writing-agent-scroll-bottom-dots");
    const arrow = bottom.locator(".writing-agent-scroll-bottom-arrow");
    await expect(dots).toBeVisible();
    await expect(dots.locator("span")).toHaveCount(3);
    for (const point of await dots.locator("span").all()) {
      await expect(point).toHaveCSS("width", "3px");
      await expect(point).toHaveCSS("height", "3px");
    }
    await expect(arrow).toBeHidden();
    const wave = await scroller.evaluate(element => {
      const points = [...element.closest(".writing-agent-panel")!.querySelectorAll<HTMLElement>(".writing-agent-scroll-bottom .wave-dots > span")];
      const animations = points.map(point => point.getAnimations()[0]!);
      const height = element.scrollHeight;
      const timing = animations.map(animation => animation.effect!.getTiming());
      animations.forEach(animation => { animation.currentTime = 108.5; });
      const firstPeak = new DOMMatrix(getComputedStyle(points[0]!).transform).m42;
      animations.forEach(animation => { animation.currentTime = 250; });
      const firstFrame = points.map(point => getComputedStyle(point).transform);
      animations.forEach(animation => { animation.currentTime = 650; });
      const secondFrame = points.map(point => getComputedStyle(point).transform);
      const restFrames = [750, 950].map(time => {
        animations.forEach(animation => { animation.currentTime = time; });
        return points.map(point => new DOMMatrix(getComputedStyle(point).transform).m42);
      });
      return { timing, firstPeak, firstFrame, secondFrame, restFrames, height, afterHeight: element.scrollHeight };
    });
    expect(wave.timing.map(timing => timing.duration)).toEqual([1000, 1000, 1000]);
    expect(wave.timing.map(timing => timing.delay)).toEqual([0, 93, 186]);
    expect(wave.firstPeak).toBeCloseTo(0.8, 4);
    expect(new Set(wave.firstFrame).size).toBeGreaterThan(1);
    expect(wave.secondFrame).not.toEqual(wave.firstFrame);
    for (const frame of wave.restFrames) for (const position of frame) expect(position).toBeCloseTo(0, 5);
    expect(wave.afterHeight).toBe(wave.height);
    const runningBounds = await bottom.boundingBox();
    await page.locator(".writing-agent-panel").screenshot({ path: `.tmp/agent-scroll-bottom-dots-${width}.png` });
    await bottom.hover();
    await expect(dots).toBeHidden();
    await expect(arrow).toBeVisible();
    expect(await bottom.boundingBox()).toEqual(runningBounds);
    await page.locator(".writing-agent-panel").screenshot({ path: `.tmp/agent-scroll-bottom-hover-${width}.png` });
    await page.mouse.move(0, 0);
    await expect(dots).toBeVisible();
    await expect(arrow).toBeHidden();
    await page.emulateMedia({ reducedMotion: "reduce" });
    for (const point of await dots.locator("span").all()) {
      await expect(point).toHaveCSS("animation-name", "none");
      await expect(point).toHaveCSS("transform", "none");
    }
    await page.getByRole("button", { name: "停止生成", exact: true }).click();
    await expect(dots).toHaveCount(0);
    await expect(arrow).toBeVisible();
    await bottom.click();
    await expect(bottom).toHaveCount(0);
  });
}
