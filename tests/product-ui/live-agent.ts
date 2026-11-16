import {expect, type Page} from "@playwright/test";

/** Open the actual App and change only its synthetic data/lifecycle fixture. */
export async function openLiveAgent(page: Page, response?: {text: string; process?: boolean}) {
  await page.goto("/?preview=app");
  const input = page.getByRole("textbox", {name: "给研究助手发送消息", exact: true});
  const launcher = page.getByRole("button", {name: "打开对话", exact: true});
  await expect(launcher.or(input)).toBeVisible();
  if (await launcher.isVisible()) await launcher.click();
  await page.evaluate(async replacement => {
    const fixture = await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx");
    fixture.configureLiveAgentResponse(replacement);
  }, response);
  return input;
}

export async function sendLiveAgentMessage(page: Page, text: string, turnCount: number) {
  await page.getByRole("textbox", {name: "给研究助手发送消息", exact: true}).fill(text);
  await page.getByRole("button", {name: "发送消息", exact: true}).click();
  await expect.poll(async () => (await liveAgentTurnIds(page)).length).toBe(turnCount);
  await expect(page.getByRole("button", {name: "停止生成", exact: true})).toHaveCount(0);
  const markdown = page.locator(".writing-agent-message.assistant .writing-agent-markdown").last();
  await expect(markdown).toHaveAttribute("aria-busy", "false");
  await expect(markdown).not.toHaveAttribute("data-stream-revealing", "true");
}

export async function holdLiveAgentOutput(page: Page, delayMs = 60_000) {
  await page.evaluate(async delay => {
    (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).holdLiveAgentOutput(delay);
  }, delayMs);
}

/** Journal counts remain independent of the production transcript window. */
export async function liveAgentTurnIds(page: Page): Promise<string[]> {
  return page.evaluate(async () => {
    return (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).liveAgentTurnIds();
  });
}
