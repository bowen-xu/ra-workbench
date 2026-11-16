import { expect, test } from "@playwright/test";

test("the production App copies bounded failure diagnostics in normal mode", async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem("research-assistant.developer-mode", "false");
    Object.defineProperty(navigator, "clipboard", { configurable: true, value: {
      writeText: async (value: string) => { sessionStorage.setItem("test-copied-diagnostic", value); },
    } });
  });
  await page.goto("/?preview=app");
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  // Inject a failed terminal state at the mock history gateway, preserving the
  // production session, transcript, message projection and copy component.
  await page.evaluate(async () => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).configureFailedAgentHistory());
  await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill("合成失败诊断");
  await page.getByRole("button", { name: "发送消息", exact: true }).click();
  const copy = page.locator('.writing-agent-message.failure[role="alert"]')
    .getByRole("button", { name: "复制诊断信息", exact: true });
  await expect(copy).toBeVisible();
  await copy.click();
  await expect(copy).toHaveText("已复制");
  const copied = await page.evaluate(() => sessionStorage.getItem("test-copied-diagnostic"));
  expect(copied).toContain("Agent 请求失败");
  expect(copied).toContain('"contextSnapshotId"');
  const data = JSON.parse(copied!.split("\n\n运行诊断：\n")[1]!);
  expect(data.schemaVersion).toBe(1);
  expect(data.conversationId).toBeTruthy();
  expect(data.turnId).toBeTruthy();
  expect(data.contextSnapshotId).toBeTruthy();
  expect(data).not.toHaveProperty("messages");
  expect(data).not.toHaveProperty("input");
  await page.mouse.move(0, 0);
  await expect(copy).toBeVisible();
  expect(copied).not.toContain("合成失败诊断");
  await page.locator(".writing-agent-panel").screenshot({ path: ".tmp/agent-failure-copy-app.png" });
});
