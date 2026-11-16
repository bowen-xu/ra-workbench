import { expect, test } from "@playwright/test";
import {liveAgentTurnIds, openLiveAgent, sendLiveAgentMessage} from "./live-agent";

for (const width of [632, 1192]) {
  test(`completed turn headers retain their position when toggled at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 808 });
    await openLiveAgent(page, {text: Array.from({length: 24}, (_, index) => `合成处理记录 ${index + 1}。\n\n`).join(""), process: true});
    for (let turn = 1; turn <= 3; turn += 1) await sendLiveAgentMessage(page, `锚点验收第 ${turn} 轮\n` + Array.from({length: 20}, (_, index) => `第 ${index + 1} 项：检查处理记录与结论。`).join("\n"), turn);
    await page.clock.install();
    const ids = await liveAgentTurnIds(page);
    expect(ids).toHaveLength(3);
    const scroller = page.locator(".writing-agent-messages");
    for (const [index, id] of [ids[0]!, ids.at(-1)!].entries()) {
      await scroller.evaluate((element, first) => {element.scrollTop = first ? 0 : element.scrollHeight; element.dispatchEvent(new Event("scroll"));}, index === 0);
      await page.clock.runFor(100);
      const header = page.locator(`[data-agent-turn-id="${id}"] > .writing-agent-turn-process > .writing-agent-turn-header`);
      await header.scrollIntoViewIfNeeded();
      await page.clock.runFor(100);
      const collapsedY = (await header.boundingBox())!.y;
      await header.click();
      await page.clock.runFor(200);
      expect((await header.boundingBox())!.y).toBeCloseTo(collapsedY, 0);
      await expect(header.locator("..")).toHaveAttribute("open", "");
      // Place an expanded header well inside the viewport. Collapsing the last
      // turn then needs a reserved range because its following content is short.
      await header.evaluate(element => {
        const list = element.closest<HTMLElement>(".writing-agent-messages")!;
        list.scrollTop += element.getBoundingClientRect().top - list.getBoundingClientRect().top - 80;
      });
      await page.clock.runFor(100);
      await header.focus();
      const expandedY = (await header.boundingBox())!.y;
      await page.keyboard.press("Enter");
      await page.clock.runFor(200);
      expect((await header.boundingBox())!.y).toBeCloseTo(expandedY, 0);
      await expect(header.locator("..")).not.toHaveAttribute("open", "");
    }
    await scroller.evaluate(element => {element.scrollTop = 0; element.dispatchEvent(new Event("scroll"));});
    await page.clock.runFor(100);
    await page.locator(".writing-agent-panel").getByRole("button", { name: "回到底部", exact: true }).click();
    await page.clock.runFor(100);
    expect(await page.locator(".writing-agent-transcript").evaluate(element => (element as HTMLElement).style.minHeight)).toBe("");
    expect(await scroller.evaluate(element => element.scrollHeight - element.clientHeight - element.scrollTop)).toBeLessThanOrEqual(6);
  });
}
