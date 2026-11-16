import { expect, type Page } from "@playwright/test";

/** Build queue fixtures through the same composer used in the application. */
export async function enqueueCanvasMessages(page: Page, messages: readonly string[]): Promise<void> {
  const input = page.getByRole("textbox", { name: /^给(?:研究助手| AI 写作助手)发送消息$/u });
  await expect(page.getByRole("button", { name: "停止生成", exact: true })).toBeVisible();
  const rows = page.locator(".agent-input-queue-body li");
  // Each viewport starts from the current App conversation data.
  while (await rows.count()) {
    const count = await rows.count();
    await rows.first().hover();
    await rows.first().getByRole("button", { name: "移除第 1 条消息", exact: true }).click();
    await expect(rows).toHaveCount(count - 1);
  }
  for (const [index, message] of messages.entries()) {
    await input.fill(message);
    await input.press("Enter");
    const row = rows.nth(index);
    await expect(row).toContainText(message);
  }
}
