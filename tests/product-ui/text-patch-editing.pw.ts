import { expect, test, type Page } from "@playwright/test";

const route = "/?preview=app&demo=text-patch";
function body(page: Page) { return page.locator(".notes-content").getByLabel("笔记正文", { exact: true }); }
async function modify(page: Page, message: string) {
  await expect(page.getByRole("textbox", { name: "笔记标题", exact: true })).toHaveValue("研究笔记");
  await expect(body(page).locator(".bn-editor")).toBeEditable();
  const processes = page.locator(".writing-agent-turn-process"), previous = await processes.count();
  await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill(message);
  await page.getByRole("button", { name: "发送消息", exact: true }).click();
  await expect(processes).toHaveCount(previous + 1);
  await expect(processes.last().locator("summary").first()).toHaveAttribute("aria-label", /已完成/);
  await page.getByRole("button", { name: "隐藏对话", exact: true }).click();
  await expect(page.getByRole("button", { name: "打开对话", exact: true })).toBeVisible();
}

test("pending Diff typing stays editable across autosaves and preserves the latest undo after reopen", async ({ page }) => {
  await page.goto(route); await modify(page, "改写研究问题");
  const editor = body(page).locator(".bn-editor");
  const paragraph = body(page).locator('[data-id="preview-argument"] .bn-inline-content').first();
  const view = await editor.elementHandle();
  // Observe the public editing attribute, including a transient lock that may
  // have disappeared before a normal toHaveAttribute assertion could see it.
  await editor.evaluate(element => {
    element.setAttribute("data-observed-editing-lock", "false");
    new MutationObserver(records => {
      if (records.some(record => record.oldValue === "false") || element.getAttribute("contenteditable") === "false")
        element.setAttribute("data-observed-editing-lock", "true");
    }).observe(element, { attributes: true, attributeFilter: ["contenteditable"], attributeOldValue: true });
  });
  await paragraph.click(); await page.keyboard.press("End");
  await expect(editor).toBeFocused();
  // Continuous typing stays below the three-second idle save boundary.
  await page.keyboard.type(" abcdef", { delay: 400 });
  await expect(paragraph).toContainText(" abcdef");
  await expect(editor).toBeFocused();
  await expect(editor).toHaveAttribute("data-observed-editing-lock", "false");
  await expect(body(page).locator(".note-diff-deleted-blocks")).not.toContainText("abcdef");
  // Undo typing and flush the resulting D by reading through chat. Native
  // history may group adjacent keystrokes even when autosave runs between them.
  await page.keyboard.press("ControlOrMeta+z");
  await expect(paragraph).not.toContainText("abcdef");
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  await modify(page, "读取笔记");
  expect(await view?.evaluate(element => element.isConnected)).toBe(true);
  const final = await paragraph.textContent();
  await page.reload();
  await expect(body(page).locator(".note-diff-chunk-actions")).toHaveCount(1);
  await expect(paragraph).toHaveText(final!);
});

test("code and quote edits stay in their live side and can be kept with the same editor", async ({ page }) => {
  await page.goto(route); await modify(page, "修改格式示例");
  const note = body(page), editor = note.locator(".bn-editor");
  const view = await editor.elementHandle();
  for (const [id, selector, suffix] of [
    ["preview-format-quote", "blockquote", " 引用补充"],
    ["preview-format-code", "pre > code", " # manual"],
  ]) {
    const content = note.locator(`[data-id="${id}"] ${selector}`).first();
    await content.scrollIntoViewIfNeeded();
    const bounds = await content.boundingBox();
    if (!bounds) throw new Error(`Missing ${id}`);
    await content.click({ position: { x: Math.min(180, bounds.width - 2), y: bounds.height - 4 } });
    await page.keyboard.press("End"); await page.keyboard.insertText(suffix!);
    await expect(content).toContainText(suffix!);
    await expect(content.locator(".note-diff-deleted-content")).not.toContainText(suffix!);
    await expect(editor).toBeFocused();
  }
  await page.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(note.locator(".note-diff-chunk-actions")).toHaveCount(0);
  await expect(note.locator('[data-id="preview-format-quote"] blockquote')).toContainText("引用补充");
  await expect(note.locator('[data-id="preview-format-code"] pre > code')).toContainText("# manual");
  expect(await view?.evaluate(element => element.isConnected)).toBe(true);
});

test("pending Diff reports queued saves and retains enabled action nodes through typing and receipts", async ({ page }) => {
  await page.goto(route); await modify(page, "改写研究问题");
  const note = body(page), status = page.locator(".notes-content .note-save-state");
  const keep = note.getByRole("button", { name: "保留第 1 处变更", exact: true });
  const action = await keep.elementHandle();
  await keep.evaluate(element => {
    element.setAttribute("data-observed-disable", "false");
    new MutationObserver(records => {
      if (records.some(record => record.oldValue !== null) || element.hasAttribute("disabled"))
        element.setAttribute("data-observed-disable", "true");
    }).observe(element, { attributes: true, attributeFilter: ["disabled"], attributeOldValue: true });
  });
  const paragraph = note.locator('[data-id="preview-argument"] .bn-inline-content').first();
  await paragraph.click(); await page.keyboard.press("End");
  await page.keyboard.insertText(" stable");
  await expect(status).toHaveAttribute("aria-label", "等待保存");
  await page.keyboard.type("abcdef", { delay: 400 });
  await expect(paragraph).toContainText(" stableabcdef");
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  await modify(page, "读取笔记");
  await expect(status).toHaveAttribute("aria-label", "已保存");
  expect(await action?.evaluate(element => element.isConnected)).toBe(true);
  await expect(keep).toBeEnabled(); await expect(keep).toHaveAttribute("data-observed-disable", "false");
  await keep.click();
  await expect(note.locator(".note-diff-chunk-actions")).toHaveCount(0);
  await expect(paragraph).toContainText(" stableabcdef");
});

test("reviewed note saves use the configured preview delay and show saving before acknowledgment", async ({ page }) => {
  await page.goto(route);
  await expect(body(page).locator(".bn-editor")).toBeEditable();
  await page.evaluate(async () => {
    const source = "/ra-workbench/apps/web/src/core/runtime/appRuntime.ts";
    const runtime = await import(source);
    runtime.getAppRuntime().preview.configure({ delayMs: 600, nextFailure: null });
  });
  await modify(page, "改写研究问题");
  const note = body(page), status = page.locator(".notes-content .note-save-state");
  const paragraph = note.locator('[data-id="preview-argument"] .bn-inline-content').first();
  await paragraph.click(); await page.keyboard.press("End");
  await page.keyboard.insertText(" 保存进度");
  await expect(status).toHaveAttribute("aria-label", "等待保存");
  await expect(status).toHaveAttribute("aria-label", "正在保存…");
  await expect(note.locator(".bn-editor")).toBeEditable();
  await expect(status).toHaveAttribute("aria-label", "已保存");
  await expect(note.locator(".note-diff-chunk-actions")).toHaveCount(1);
  await expect(paragraph).toContainText("保存进度");
});
