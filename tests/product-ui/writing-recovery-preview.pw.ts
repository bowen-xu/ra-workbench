import { expect, test } from "@playwright/test";
import { writingRecoveryPreviewUrl } from "../../apps/web/src/core/runtime/writingRecoveryPreview";

const cases = [
  { id: "discovery", label: "文件修改读取失败", message: "文件修改读取失败", action: "重新读取" },
  { id: "stale", label: "旧修改版本冲突", message: "旧修改与当前稿件版本不一致", action: "保留当前稿件，放弃这次修改" },
  { id: "application", label: "文件应用结果未确认", message: "接收文件修改尚未确认", action: "核对文件结果" },
  { id: "acknowledgement", label: "交付记录确认失败", message: "交付记录待确认", action: "重试确认记录" },
] as const;

for (const scenario of cases) test(`preview injects ${scenario.id} through production recovery and preserves the result on reload`, async ({ page }) => {
  await page.goto(writingRecoveryPreviewUrl(scenario.id));
  await expect(page.getByRole("button", { name: "模拟写作错误", exact: true })).toHaveCount(0);
  await expect(page).toHaveURL(new RegExp(`writingRecovery=${scenario.id}`));
  const manuscript = page.getByRole("button", { name: "Autonomy（合成稿件） main.tex", exact: true });
  await manuscript.click();
  await expect(page.locator("[data-sonner-toast]").filter({ hasText: scenario.message })).toBeVisible({ timeout: 15_000 });
  await expect(page.locator(".recovery-banner")).toHaveCount(0);
  const editor = page.locator(".writing-editor-host .cm-content");
  const originalEditor = await editor.elementHandle();
  const text = await editor.textContent();
  const feedback = page.locator(".writing-file-recovery-stack");
  await feedback.locator("summary").click();
  await feedback.getByRole("button", { name: scenario.action, exact: true }).click();
  if (scenario.id === "application") {
    await feedback.getByRole("button", { name: "重试保存文件结果", exact: true }).click();
  }
  await expect(feedback).toBeHidden();
  expect(await originalEditor!.evaluate(node => node.isConnected)).toBe(true);
  if (scenario.id !== "discovery") await expect(editor).toHaveText(text ?? "");
  await expect(page.locator(".writing-workbench .save-state")).toHaveText("已保存");
  // Discovery recovery can hide its error before the seeded file delivery finishes.
  await expect.poll(() => page.evaluate(id => {
    const prefix = `research-assistant.writing-recovery-preview.v3:${id}:`;
    const manuscriptId = localStorage.getItem(prefix + "initial-manuscript");
    const raw = localStorage.getItem(prefix + "agent:" + manuscriptId);
    return raw ? JSON.parse(raw).agent.completions.entries[0]?.[1].acknowledged : false;
  }, scenario.id), { timeout: 15000 }).toBe(true);
  await page.reload();
  await manuscript.click();
  await expect(editor).toBeVisible();
  await expect(feedback).toBeHidden();
  await expect(page.locator("[data-sonner-toast]").filter({ hasText: scenario.message })).toHaveCount(0);
  await page.screenshot({ path: `.tmp/writing-recovery-${scenario.id}.png` });
});
