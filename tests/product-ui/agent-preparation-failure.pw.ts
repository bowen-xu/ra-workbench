import { expect, test } from "@playwright/test";
import {liveAgentTurnIds, openLiveAgent, holdLiveAgentOutput} from "./live-agent";

const failure = "请求核对失败，消息已保留在输入框中，请检查后重新发送。";

for (const failed of [false, true]) {
  test(`the production App keeps request waiting in the composer (failure: ${failed})`, async ({page}) => {
    const input = await openLiveAgent(page);
    await holdLiveAgentOutput(page);
    await page.evaluate(async failed => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).holdAgentMetadata(failed), failed);
    const send = page.getByRole("button", {name: "发送消息", exact: true});
    await input.fill("等待会话核对"); await send.click();
    const status = page.getByRole("status", {name: "输入框准备状态", exact: true});
    await expect(status).toHaveText("正在核对请求……");
    await expect(page.locator('.agent-composer-input-state [aria-label="输入框准备状态"]')).toHaveCount(1);
    await expect(page.locator('.writing-agent-messages [aria-label="输入框准备状态"]')).toHaveCount(0);
    await expect(page.getByRole("button", {name: "停止发送", exact: true})).toBeEnabled();
    await expect(page.locator('[data-pending-submission="true"]')).toHaveText("等待会话核对");
    await expect(page.getByRole("button", {name: "取消发送", exact: true})).toHaveCount(0);
    await page.screenshot({path: `.tmp/agent-send-wait-app-${failed}.png`});
    await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).releaseAgentMetadata());
    if (failed) {
      await expect(page.locator('[data-sonner-toast]')).toContainText("请求核对失败");
      await expect(input).toBeEnabled(); await expect(input).toHaveText("等待会话核对");
      await expect(page.getByRole("button", {name: "重新核对对话状态", exact: true})).toHaveCount(0);
      await expect(page.locator('.writing-agent-message.user')).toHaveCount(0);
      await input.fill("修改后重新核对并发送"); await send.click();
    }
    await expect(page.getByRole("button", {name: "停止生成", exact: true})).toBeVisible();
    await expect(page.locator('.writing-agent-message.user')).toHaveCount(1);
    await expect(status).toHaveCount(0);
  });
}

test("full App shares preparation failure feedback and permits manual resend", async ({page}) => {
  await page.goto("/?preview=app");
  await page.getByRole("button", {name: "打开对话", exact: true}).click();
  await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).configureAgentPreview({delayMs: 0, nextFailure: "agent"}));
  const input = page.getByRole("textbox", {name: "给研究助手发送消息", exact: true});
  const send = page.getByRole("button", {name: "发送消息", exact: true});
  await input.fill("失败后由我修改重发");
  await send.click();
  await expect(page.locator("[data-sonner-toast]")).toContainText(failure);
  await expect(input).toHaveText("失败后由我修改重发");
  await expect(send).toBeEnabled();
  await expect(page.getByRole("region", {name: "发送恢复", exact: true})).toHaveCount(0);
  await expect(page.locator(".writing-agent-message.user")).toHaveCount(0);
  await page.locator(".writing-agent-panel").screenshot({path: ".tmp/agent-preparation-failed-app.png"});
  await input.fill("现在重新发送");
  await send.click();
  await expect(page.locator(".writing-agent-message.assistant")).toContainText("本轮已读取");
  await expect(page.locator(".writing-agent-message.user")).toHaveCount(1);
  await expect(page.locator(".writing-agent-message.user")).toContainText("现在重新发送");
});

for(const lostReceipt of [false,true])test(`the production App preserves an uncertain start and settles a confirmed lost receipt once (lost receipt: ${lostReceipt})`,async({page})=>{
  await page.goto("/?preview=app");
  await page.getByRole("button",{name:"打开对话",exact:true}).click();
  await page.evaluate(async lost => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).installAgentStartFailure(lost), lostReceipt);
  const input=page.getByRole("textbox",{name:"给研究助手发送消息",exact:true});
  const send=page.getByRole("button",{name:"发送消息",exact:true});
  await input.fill("首次发送");await send.click();
  const recover = page.getByRole("button", {name: "核对原请求", exact: true});
  await expect(recover).toBeVisible();
  await expect(page.locator('[data-sonner-toast]')).toContainText("CONTEXT_PREPARE_UNCONFIRMED");
  await recover.click();
  const launcher = page.getByRole("button", {name: "打开对话", exact: true});
  if (await launcher.isVisible()) await launcher.click();
  const calls = await page.evaluate(() => JSON.parse(sessionStorage.getItem("test-start-requests") ?? "[]") as string[]);
  const status = await page.evaluate(() => JSON.parse(sessionStorage.getItem("test-start-status-requests") ?? "[]") as string[]);
  expect(calls).toHaveLength(1);
  expect(status).toContain(calls[0]);
  if (lostReceipt) {
    await expect(page.locator('.writing-agent-message.assistant')).toHaveCount(1);
    await expect(page.locator('.writing-agent-message.user')).toHaveCount(1);
    await expect(input).toBeEmpty();
    await expect(page.getByRole("button", {name: "核对原请求", exact: true})).toHaveCount(0);
  } else {
    await expect(input).toHaveText("首次发送");
    await expect(page.locator('.writing-agent-message.assistant')).toHaveCount(0);
    expect(await liveAgentTurnIds(page)).toEqual([]);
    await expect(page.locator('[data-sonner-toast]')).toContainText("mock_worker_unconfirmed");
    await expect(page.getByRole("button", {name: "核对原请求", exact: true})).toBeVisible();
  }
  await expect(page.getByRole("region", {name: "发送恢复", exact: true})).toHaveCount(0);
  await expect(page.getByRole("button", {name: "核对并重试同一请求", exact: true})).toHaveCount(0);

});
