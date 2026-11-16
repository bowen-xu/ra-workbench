import { expect, test } from "@playwright/test";

test("file recovery failures remain usable without covering editing or manuscript navigation", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await page.getByRole("button", { name: "新建稿件", exact: true }).click();
  await page.getByRole("dialog", { name: "新建 LaTeX 稿件" }).getByRole("textbox").fill("恢复提示布局");
  await page.getByRole("button", { name: "创建稿件", exact: true }).click();
  const editor = page.locator(".writing-editor-host .cm-content");
  await expect(editor).toBeVisible();
  const originalEditor = await editor.elementHandle();
  // The browser writing gateway is available; native file-operation discovery
  // is unavailable. Exercise the real production error/retry components.
  const feedback = page.locator(".writing-file-recovery-stack");
  await expect(page.locator("[data-sonner-toast]").filter({ hasText: "读取失败" })).toBeVisible();
  await expect(feedback.locator("summary")).toBeVisible();
  await expect(page.locator(".recovery-banner")).toHaveCount(0);
  const notice = page.locator("[data-sonner-toast]").filter({ hasText: "读取失败" });
  const closeNotice = notice.getByRole("button", { name: "关闭通知" });
  await closeNotice.focus();
  await closeNotice.press("Enter");
  await expect(notice).toBeHidden();
  await expect(feedback.locator(".writing-file-recovery-actions").first()).toBeHidden();
  await feedback.locator("summary").click();
  await expect(feedback.getByRole("button", { name: "重新读取", exact: true })).toBeVisible();
  await feedback.locator("summary").click();
  await expect(feedback.locator(".writing-file-recovery-actions").first()).toBeHidden();
  await page.getByRole("button", { name: "编译选项", exact: true }).click();
  await expect(page.getByRole("group", { name: "Code Check" })).toBeVisible();
  await page.keyboard.press("Escape");
  await page.setViewportSize({ width: 811, height: 808 });
  const feedbackBounds = await feedback.boundingBox();
  const workbenchBounds = await page.locator(".writing-workbench").boundingBox();
  const toolbarBounds = await page.locator(".writing-toolbar").boundingBox();
  const editorBounds = await editor.boundingBox();
  expect(feedbackBounds).not.toBeNull();
  expect(workbenchBounds).not.toBeNull();
  expect(feedbackBounds!.y).toBeGreaterThanOrEqual(workbenchBounds!.y);
  expect(feedbackBounds!.y + feedbackBounds!.height).toBeLessThanOrEqual(toolbarBounds!.y + toolbarBounds!.height + 1);
  expect(feedbackBounds!.y + feedbackBounds!.height).toBeLessThanOrEqual(editorBounds!.y + 1);
  await page.getByRole("tablist", { name: "打开的标签页" }).getByRole("tab", { name: "main.tex", exact: true }).click();
  await editor.click();
  await page.keyboard.insertText("ordinary input during discovery failure");
  await expect(editor).toContainText("ordinary input during discovery failure");
  expect(await originalEditor!.evaluate((node) => node.isConnected)).toBe(true);
  await page.getByRole("tab", { name: "返回写作项目列表", exact: true }).click();
  await expect(page.getByRole("heading", { name: "我的稿件", exact: true })).toBeVisible();
  await expect(feedback).toHaveCount(0);
  await page.getByRole("button", { name: "恢复提示布局 main.tex", exact: true }).click();
  await expect(feedback.locator('[aria-label="读取 Agent 文件修改"] button')).toHaveCount(1);
  await expect(editor).toContainText("ordinary input during discovery failure");
  expect(await originalEditor!.evaluate((node) => node.isConnected)).toBe(true);
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await expect(editor).toBeVisible();
  await expect(editor).toContainText("ordinary input during discovery failure");
  await page.screenshot({ path: ".tmp/writing-recovery-layout.png" });
});

for (const route of ["?demo=app", "?preview=app"]) test(`${route} confines retained manuscript recovery to its editing workbench`, async ({ page }) => {
  await page.goto(`/${route}`);
  await page.getByRole("button", { name: "写作", exact: true }).click();
  const feedback = page.locator(".writing-file-recovery-stack");
  const manuscript = page.getByRole("button", { name: "Autonomy（合成稿件） main.tex", exact: true });
  await expect(manuscript).toBeVisible();
  await expect(feedback).toHaveCount(0);
  await manuscript.click();
  const editor = page.locator(".writing-editor-host .cm-content");
  await expect(editor).toBeVisible();
  await expect(feedback).toHaveCount(1);
  // The preview applies a synthetic proposal after mounting the editor. Finish
  // its review before exercising the normal save-and-return boundary.
  await page.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(page.getByRole("button", { name: "Keep all", exact: true })).toHaveCount(0);
  await expect(page.locator(".writing-workbench .save-state")).toHaveText("已保存");
  const originalEditor = await editor.elementHandle();
  await page.getByRole("tab", { name: "返回写作项目列表", exact: true }).click();
  await expect(manuscript).toBeVisible();
  await expect(feedback).toHaveCount(0);
  expect(await originalEditor!.evaluate(node => node.isConnected)).toBe(true);
  await page.screenshot({ path: `.tmp/writing-recovery-list-${route.includes("demo") ? "demo" : "preview"}.png` });
  await manuscript.click();
  await expect(editor).toBeVisible();
  await expect(feedback).toHaveCount(1);
  expect(await originalEditor!.evaluate(node => node.isConnected)).toBe(true);
  await page.reload();
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await manuscript.click();
  await expect(editor).toBeVisible();
  await expect(page.locator(".writing-workbench .save-state")).toHaveText("已保存");
  await expect(feedback.locator(".writing-file-recovery-actions")).toHaveCount(0);
});

test("editing an unaccepted Agent diff survives closing and reopening without replaying delivery", async ({ page, context }) => {
  await page.goto("/?demo=app");
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await page.getByRole("button", { name: "Autonomy（合成稿件） main.tex", exact: true }).click();
  const editor = page.locator(".writing-editor-host .cm-content");
  await expect(page.getByRole("button", { name: "Keep all", exact: true })).toBeEnabled();
  await editor.locator(".cm-line").filter({ hasText: "The proposed framework clarifies how value information is represented and updated." }).click();
  await editor.press("Home");
  await page.keyboard.insertText("% user revised the Agent diff: ");
  await expect(editor).toContainText("user revised the Agent diff");
  await expect(page.getByRole("button", { name: "Keep all", exact: true })).toBeVisible();
  await expect(page.locator(".writing-workbench .save-state")).toHaveText("已保存");
  await page.close();

  const reopened = await context.newPage();
  await reopened.goto("/?demo=app");
  await reopened.getByRole("button", { name: "写作", exact: true }).click();
  await reopened.getByRole("button", { name: "Autonomy（合成稿件） main.tex", exact: true }).click();
  await expect(reopened.locator(".writing-editor-host .cm-content")).toContainText("user revised the Agent diff");
  await expect(reopened.getByRole("button", { name: "Keep all", exact: true })).toBeEnabled();
  await expect(reopened.locator(".writing-workbench .save-state")).toHaveText("已保存");
  await expect(reopened.locator(".writing-file-recovery-stack .writing-file-recovery-actions")).toHaveCount(0);
  await reopened.screenshot({ path: ".tmp/writing-diff-reopen.png" });
});

test("a stale preview proposal can be inspected and abandoned without changing the current manuscript", async ({ page }) => {
  await page.goto("/?demo=app");
  await page.getByRole("button", { name: "写作", exact: true }).click();
  const manuscript = page.getByRole("button", { name: "Autonomy（合成稿件） main.tex", exact: true });
  await manuscript.click();
  await page.getByRole("button", { name: "Keep all", exact: true }).click();
  await expect(page.getByRole("button", { name: "Keep all", exact: true })).toHaveCount(0);
  await expect(page.locator(".writing-workbench .save-state")).toHaveText("已保存");
  const content = await page.locator(".writing-editor-host .cm-content").textContent() ?? "";
  expect(content).toContain("This paper presents");
  // Isolated old-format fixture: a pending proposal survives alongside a newer
  // manuscript, with no application journal. Do not infer a successful delivery
  // from matching editor text or invent a historical application receipt.
  const frozen = await page.evaluate(() => {
    const prefix = "research-assistant.app-preview.v3:";
    const id = localStorage.getItem(prefix + "initial-manuscript");
    const raw = localStorage.getItem(prefix + "agent:" + id);
    if (!raw) throw new Error("Missing synthetic Agent state");
    return JSON.stringify(JSON.parse(raw).agent.completions.entries[0][1].texts);
  });
  // Inject after the old runtime's final persistence, before the new runtime
  // restores its state. Never reinject after the user rejects the proposal.
  await page.addInitScript(() => {
    if (sessionStorage.getItem("stale-delivery-fixture-injected")) return;
    const prefix = "research-assistant.app-preview.v3:";
    const id = localStorage.getItem(prefix + "initial-manuscript");
    const key = prefix + "agent:" + id;
    const raw = localStorage.getItem(key);
    if (!raw) throw new Error("Missing synthetic Agent state");
    const state = JSON.parse(raw), entry = state.agent.completions.entries[0][1];
    entry.application = null; entry.applicationId = null; entry.acknowledged = false;
    localStorage.setItem(key, JSON.stringify(state));
    sessionStorage.setItem("stale-delivery-fixture-injected", "1");
  });
  await page.reload();
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await manuscript.click();
  const feedback = page.locator(".writing-file-recovery-stack");
  await expect(page.locator("[data-sonner-toast]").filter({ hasText: "旧修改与当前稿件版本不一致" })).toBeVisible();
  await expect(feedback).not.toContainText("旧修改与当前稿件版本不一致");
  await expect(feedback.locator(".writing-file-recovery-actions")).toBeHidden();
  await page.screenshot({ path: ".tmp/writing-stale-proposal-collapsed.png" });
  await feedback.locator("summary").click();
  await expect(feedback.locator(".writing-file-recovery-actions")).toBeVisible();
  await page.screenshot({ path: ".tmp/writing-stale-proposal.png" });
  await page.getByRole("button", { name: "查看来源对话", exact: true }).click();
  await expect(page.locator(".writing-agent-panel:visible")).toBeVisible();
  await page.getByRole("button", { name: /^隐藏(?: AI 写作助手|对话)$/ }).click();
  await page.getByRole("button", { name: "保留当前稿件，放弃这次修改", exact: true }).click();
  await expect(feedback).toBeHidden();
  await expect(page.locator(".writing-editor-host .cm-content")).toHaveText(content);
  await page.reload();
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await manuscript.click();
  await expect(page.locator(".writing-editor-host .cm-content")).toHaveText(content);
  await expect(feedback).toBeHidden();
  expect(await page.evaluate(() => {
    const prefix = "research-assistant.app-preview.v3:";
    const id = localStorage.getItem(prefix + "initial-manuscript");
    const entry = JSON.parse(localStorage.getItem(prefix + "agent:" + id) ?? "{}").agent.completions.entries[0][1];
    return { rejected: entry.rejected, frozen: JSON.stringify(entry.texts) };
  })).toEqual({ rejected: true, frozen });
});
