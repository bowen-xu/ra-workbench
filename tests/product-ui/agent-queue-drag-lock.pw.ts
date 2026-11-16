import { expect, test } from "@playwright/test";
import { enqueueCanvasMessages } from "./agent-canvas-queue";
import {openLiveAgent, holdLiveAgentOutput} from "./live-agent";

for (const width of [1172, 685]) {
  test(`dragging locks and hides other queue handles until release or cancellation at ${width}px`, async ({ page }) => {
    await page.setViewportSize({ width, height: 808 });
    await openLiveAgent(page);
    await holdLiveAgentOutput(page);
    await page.getByRole("textbox", {name: "给研究助手发送消息", exact: true}).fill("正在执行的合成任务");
    await page.getByRole("button", {name: "发送消息", exact: true}).click();
    await enqueueCanvasMessages(page, ["修改 keywords 部分", "按百分比报告进度"]);
    const queue = page.getByRole("region", { name: "队列消息", exact: true });
    const rows = queue.locator("li");
    await expect(rows).toHaveCount(2);
    const first = rows.filter({ hasText: "修改 keywords 部分" });
    const second = rows.filter({ hasText: "按百分比报告进度" });
    const firstHandle = first.locator(".agent-input-queue-drag-handle");
    const secondHandle = second.locator(".agent-input-queue-drag-handle");
    await first.hover();
    await expect(firstHandle).toHaveCSS("opacity", "1");
    const start = await firstHandle.boundingBox();
    const target = await second.boundingBox();
    if (!start || !target) throw new Error("Queue rows unavailable");
    await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2);
    await page.mouse.down();
    // All shared reorder hosts activate after 6px, preserving ordinary clicks.
    await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2 + 6);
    await expect(secondHandle).toBeDisabled();
    // Cross the other row before moving past its midpoint to reproduce hover.
    await page.mouse.move(target.x + 45, target.y + 2);
    await second.getByRole("button", { name: "第 2 条消息菜单", exact: true }).focus();
    await expect(secondHandle).toHaveCSS("opacity", "0");
    await expect(secondHandle).toHaveCSS("pointer-events", "none");
    await expect(secondHandle).toHaveCSS("transition-duration", "0s");
    await page.screenshot({ path: `.tmp/agent-queue-drag-lock-${width}.png`, fullPage: true });
    await page.mouse.move(target.x + 45, target.y + target.height - 2);
    await page.mouse.up();
    await expect(rows.first()).toContainText("按百分比报告进度");
    await expect(secondHandle).toBeEnabled();
    await expect(queue).not.toHaveClass(/is-reordering/);
    await second.hover();
    await expect(secondHandle).toHaveCSS("opacity", "1");

    const next = await secondHandle.boundingBox();
    if (!next) throw new Error("Restored handle unavailable");
    await page.mouse.move(next.x + next.width / 2, next.y + next.height / 2);
    await page.mouse.down();
    await page.mouse.move(next.x + next.width / 2, next.y + next.height / 2 + 6);
    await expect(firstHandle).toBeDisabled();
    await secondHandle.dispatchEvent("pointercancel", { pointerId: 1, bubbles: true });
    await page.mouse.up();
    await expect(firstHandle).toBeEnabled();
    await expect(queue).not.toHaveClass(/is-reordering/);
    await first.hover();
    await expect(firstHandle).toHaveCSS("opacity", "1");
  });
}
