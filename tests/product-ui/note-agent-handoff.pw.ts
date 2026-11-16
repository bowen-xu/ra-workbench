import { expect, test } from "@playwright/test";

test("Agent command fences manual input, then exposes an editable Diff that saves and reopens", async ({ page }) => {
  test.setTimeout(60_000);
  await page.goto("/?preview=app&demo=text-patch");
  const note = page.locator(".notes-content").getByLabel("笔记正文", { exact: true });
  const editor = note.locator(".bn-editor"), status = page.locator(".notes-content .note-save-state");
  await expect(page.getByRole("textbox", { name: "笔记标题", exact: true })).toHaveValue("研究笔记");
  await expect(editor).toBeEditable();
  // Hold the mock save boundary until the input-fence assertions finish.
  // Browser screenshots and hint delays must not race the write's completion.
  await page.evaluate(async () => {
    const source = "/ra-workbench/apps/web/src/core/runtime/appRuntime.ts";
    const runtime = await import(source);
    const preview = runtime.getAppRuntime().preview;
    preview.configure({ delayMs: 0, nextFailure: null });
    const before = preview.beforeOperation.bind(preview);
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    preview.beforeOperation = async (name: string) => {
      if (name === "save") await gate;
      await before(name);
    };
    (window as Window & { releaseAgentWrite?: () => void }).releaseAgentWrite = () => {
      preview.beforeOperation = before;
      release();
    };
  });
  await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill("改写研究问题");
  await page.getByRole("button", { name: "发送消息", exact: true }).click();
  await expect(status).toHaveAttribute("aria-label", "agent正在写入", { timeout: 15_000 });
  await expect(editor).toHaveAttribute("contenteditable", "false");
  await expect(status.locator(".lucide-loader-circle")).toBeVisible();
  await status.hover();
  await expect(page.getByRole("tooltip", { name: "agent正在写入", exact: true })).toBeVisible();
  await page.screenshot({ path: ".tmp/note-agent-writing.png" });
  await editor.focus(); await page.keyboard.insertText("BLOCKED_INPUT");
  await expect(note).not.toContainText("BLOCKED_INPUT");
  await page.evaluate(() => {
    const host = window as Window & { releaseAgentWrite?: () => void };
    if (!host.releaseAgentWrite) throw new Error("Missing mock save gate");
    host.releaseAgentWrite(); delete host.releaseAgentWrite;
  });
  await expect(page.locator(".writing-agent-turn-process").last().locator("summary").first()).toHaveAttribute("aria-label", /已完成/);
  await expect(note.locator(".note-diff-chunk-actions")).toHaveCount(1);
  await expect(editor).toBeEditable();
  await page.getByRole("button", { name: "隐藏对话", exact: true }).click();
  const paragraph = note.locator('[data-id="preview-argument"] .bn-inline-content').first();
  await paragraph.click(); await page.keyboard.press("End"); await page.keyboard.insertText(" 手动补充保留");
  await expect(status).toHaveAttribute("aria-label", "等待保存");
  await expect(status).toHaveAttribute("aria-label", "已保存", { timeout: 10_000 });
  await page.evaluate(async () => {
    const source = "/ra-workbench/apps/web/src/core/runtime/appRuntime.ts";
    const runtime = await import(source);
    runtime.getAppRuntime().preview.configure({ delayMs: 100, nextFailure: null });
  });
  await page.reload();
  await expect(paragraph).toContainText("手动补充保留");
  await expect(note.locator(".note-diff-chunk-actions")).toHaveCount(1);
  await expect(editor).toBeEditable();
  await page.getByRole("button", { name: "隐藏对话", exact: true }).click();
  await page.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(note.locator(".note-diff-chunk-actions")).toHaveCount(0);
  await expect(paragraph).toContainText("手动补充保留");
});
