import {expect, test, type Page} from "@playwright/test";

async function revealChat(page: Page) {
  const launcher = page.getByRole("button", {name: "打开对话", exact: true});
  if (await launcher.isVisible()) await launcher.click();
  await expect(page.locator(".writing-agent-panel:visible")).toBeVisible();
}

test("an established conversation only recommends its active page and retains staged choices when switching tabs", async ({page}) => {
  test.setTimeout(45_000);
  await page.goto("/?preview=app");
  await page.getByRole("row", {name: /Vision by Logic/}).dblclick();
  await revealChat(page);
  const input = page.getByRole("textbox", {name: "给研究助手发送消息", exact: true});
  await input.fill("Read the current paper");
  await page.getByRole("button", {name: "发送消息", exact: true}).click();
  await expect(page.locator(".writing-agent-panel:visible")).toContainText("本轮已读取");
  await expect(page.getByRole("button", {name: "停止生成", exact: true})).toHaveCount(0);
  await page.getByRole("tab", {name: "返回文献列表", exact: true}).click();
  await page.getByRole("row", {name: /Synthetic Research Preview/}).dblclick();
  await revealChat(page);
  const synthetic = page.getByRole("group", {name: /^推荐持续上下文 Synthetic Research Preview/});
  const recommendations = page.getByRole("group", {name: /^推荐持续上下文 /});
  await expect(synthetic).toHaveAttribute("data-added", "false");

  await page.getByRole("tab", {name: /^Vision by Logic/}).click();
  await revealChat(page);
  await expect(recommendations).toHaveCount(0);
  await page.screenshot({path: ".tmp/active-context-only.png"});
  // An open background PDF must not fill the empty recommendation on the list.
  await page.getByRole("tab", {name: "返回文献列表", exact: true}).click();
  await revealChat(page);
  await expect(recommendations).toHaveCount(0);

  await page.getByRole("tab", {name: /^Synthetic Research Preview/}).click();
  await revealChat(page);
  await expect(synthetic).toHaveAttribute("data-added", "false");
  await synthetic.getByRole("button", {name: /^添加推荐上下文 /}).click();
  await expect(synthetic).toHaveAttribute("data-added", "true");
  await page.getByRole("tab", {name: /^Vision by Logic/}).click();
  await revealChat(page);
  await expect(recommendations).toHaveCount(0);
  await page.getByRole("button", {name: "上下文管理", exact: true}).click();
  const dialog = page.getByRole("dialog", {name: "上下文管理", exact: true});
  await expect(dialog.getByRole("checkbox", {name: "使用 Synthetic Research Preview: Evidence and Autonomy", exact: true})).toBeChecked();
  await page.keyboard.press("Escape");
  await page.getByRole("tab", {name: /^Synthetic Research Preview/}).click();
  await revealChat(page);
  await expect(synthetic).toHaveAttribute("data-added", "true");
  await input.fill("Read the newly selected paper");
  await page.getByRole("button", {name: "发送消息", exact: true}).click();
  await expect(recommendations).toHaveCount(0);
});
