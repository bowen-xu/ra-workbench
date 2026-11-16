import { expect, test } from "@playwright/test";
import { enqueueCanvasMessages } from "./agent-canvas-queue";
import {openLiveAgent, holdLiveAgentOutput} from "./live-agent";

test("the production App starts a selected queued input after stop and preserves the unsent draft", async ({ page }) => {
  await openLiveAgent(page);
    await holdLiveAgentOutput(page);
    await page.getByRole("textbox", {name: "给研究助手发送消息", exact: true}).fill("正在执行的合成任务");
    await page.getByRole("button", {name: "发送消息", exact: true}).click();
  await enqueueCanvasMessages(page, ["修改 keywords 部分", "按百分比报告进度"]);
  const queue = page.locator(".agent-input-queue-body");
  await expect(queue.locator("li")).toHaveCount(2);
  await page.getByRole("button", { name: "停止生成", exact: true }).click();
  const second = queue.getByRole("button", { name: "用第 2 条消息启动新轮次", exact: true });
  await expect(second).toBeEnabled();
  await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill("继续保留的未发送草稿");
  await queue.locator("li").nth(1).hover();
  await second.click();
  await expect(page.locator(".writing-agent-transcript")).toContainText("按百分比报告进度");
  await expect(queue.locator("li")).toHaveCount(1);
  await expect(queue).toContainText("修改 keywords 部分");
  await expect(queue).not.toContainText("按百分比报告进度");
  await expect(page.getByRole("textbox", { name: "给研究助手发送消息", exact: true })).toHaveText("继续保留的未发送草稿");
  await expect(queue.locator("li").first()).toContainText("修改 keywords 部分");
  await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill("");
  await expect(page.getByRole("button", { name: "停止生成", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "停止生成", exact: true }).click();
});
