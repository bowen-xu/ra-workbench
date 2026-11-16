import { expect, test, type Page } from "@playwright/test";

const route = "/?preview=app&demo=text-patch";
function body(page: Page) { return page.locator(".notes-content").getByLabel("笔记正文", { exact: true }); }
async function modify(page: Page, text = "修改格式示例") {
  await expect(page.getByRole("textbox", { name: "笔记标题", exact: true })).toHaveValue("研究笔记");
  await expect(body(page).locator(".bn-editor")).toBeEditable();
  await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill(text);
  await page.getByRole("button", { name: "发送消息", exact: true }).click();
  await expect(page.locator(".writing-agent-turn-process").last().locator("summary").first()).toHaveAttribute("aria-label", /已完成/);
  await page.getByRole("button", { name: "隐藏对话", exact: true }).click();
}

test("saved pending B/D survives reopening, partial Keep, and a later Undo all", async ({ page }) => {
  await page.goto(route);
  await modify(page);
  await expect(body(page).locator(".note-diff-chunk-actions")).toHaveCount(7);
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  await body(page).getByRole("button", { name: "保留第 1 处变更", exact: true }).click();
  await expect(body(page).locator(".note-diff-chunk-actions")).toHaveCount(6);
  await page.reload();
  await expect(body(page).locator(".note-diff-chunk-actions")).toHaveCount(6);
  await expect(body(page).locator('[data-id="preview-format-heading"] .bn-inline-content').first()).toContainText("实验指标与稳健性验证");
  await page.getByRole("button", { name: "隐藏对话", exact: true }).click();
  await page.getByRole("button", { name: "Undo all", exact: true }).click();
  await expect(body(page).locator(".note-diff-chunk-actions")).toHaveCount(0);
  await page.reload();
  await expect(body(page).locator(".note-diff-chunk-actions")).toHaveCount(0);
  await expect(body(page).locator('[data-id="preview-format-heading"] .bn-inline-content').first()).toContainText("实验指标与稳健性验证");
  await expect(body(page).locator('[data-id="preview-format-table"] table').first()).toContainText("0.65");
});

test("manual input after Keep uses the same history and saving does not prevent undoing Keep", async ({ page }) => {
  await page.goto(route);
  await modify(page, "改写研究问题");
  await expect(body(page).locator(".note-diff-chunk-actions")).toHaveCount(1);
  await page.getByRole("button", { name: "Keep all", exact: true }).click();
  const paragraph = body(page).locator('[data-id="preview-argument"] .bn-inline-content').first();
  await paragraph.click();
  await page.keyboard.press("End");
  await page.keyboard.insertText("手动补充");
  await expect(paragraph).toContainText("手动补充");
  // Reopen flushes the current D/B through the production close boundary.
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill("读取笔记");
  await page.getByRole("button", { name: "发送消息", exact: true }).click();
  await expect(page.locator(".writing-agent-turn-process").last().locator("summary").first()).toHaveAttribute("aria-label", /已完成/);
  await page.getByRole("button", { name: "隐藏对话", exact: true }).click();
  await paragraph.click();
  await page.keyboard.press("ControlOrMeta+z");
  await expect(paragraph).not.toContainText("手动补充");
  await expect(body(page).locator(".note-diff-chunk-actions")).toHaveCount(0);
  // Reading flushes the prior edit receipt; it does not install another editor
  // transaction or replace the history stack.
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill("读取笔记");
  await page.getByRole("button", { name: "发送消息", exact: true }).click();
  await expect(page.locator(".writing-agent-turn-process").last().locator("summary").first()).toHaveAttribute("aria-label", /已完成/);
  await page.getByRole("button", { name: "隐藏对话", exact: true }).click();
  await paragraph.click();
  await page.keyboard.press("ControlOrMeta+z");
  await expect(body(page).locator(".note-diff-chunk-actions")).toHaveCount(1);
  await expect(paragraph).toContainText("证据更新判断");
});
