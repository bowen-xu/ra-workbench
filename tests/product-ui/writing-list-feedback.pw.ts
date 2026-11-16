import { expect, test } from "@playwright/test";

for (const url of ["/", "/?preview=app"]) {
  test(`manuscript diagnostics belong to the visible editor on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "写作", exact: true }).click();
    const home = page.getByRole("tab", { name: "返回写作项目列表", exact: true });
    const tabs = page.getByRole("tablist", { name: "打开的标签页", exact: true });
    const warning = page.locator("[data-sonner-toast]").filter({ hasText: "缺失引用：listFeedback2026" });
    await expect(page.getByRole("heading", { name: "我的稿件", exact: true })).toBeVisible();
    await expect(tabs.getByRole("tab", { name: "main.tex", exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: "新建稿件", exact: true }).click();
    await page.getByRole("dialog", { name: "新建 LaTeX 稿件" }).getByRole("textbox").fill("List feedback regression");
    await page.getByRole("button", { name: "创建稿件", exact: true }).click();
    const editor = page.locator(".writing-editor-host .cm-content");
    await expect(editor).toContainText("\\documentclass");
    const retainedEditor = await editor.elementHandle();
    await editor.click();
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.insertText("\n\\cite{listFeedback2026}");
    await expect(warning).toBeVisible();

    await home.click();
    await expect(page.getByRole("heading", { name: "我的稿件", exact: true })).toBeVisible();
    await expect(warning).toHaveCount(0);
    await tabs.getByRole("tab", { name: "main.tex", exact: true }).click();
    await expect(warning).toBeVisible();
    expect(await retainedEditor!.evaluate(element => element.isConnected)).toBe(true);
    await expect(editor).toContainText("listFeedback2026");

    await page.getByRole("button", { name: "文献", exact: true }).click();
    await expect(warning).toHaveCount(0);
    await page.getByRole("button", { name: "写作", exact: true }).click();
    await expect(warning).toBeVisible();
    await editor.click();
    await page.keyboard.press("ControlOrMeta+z");
    await expect(editor).not.toContainText("listFeedback2026");
    await expect(warning).toHaveCount(0);

    // A saved manuscript containing missing citations must not be implicitly opened on reload.
    await page.keyboard.insertText("\n\\cite{listFeedback2026}");
    await expect(warning).toBeVisible();
    await home.click();
    await page.reload();
    await page.getByRole("button", { name: "写作", exact: true }).click();
    await expect(page.getByRole("heading", { name: "我的稿件", exact: true })).toBeVisible();
    await page.waitForTimeout(700); // Allow the debounced source index to run if incorrectly started.
    await expect(tabs.getByRole("tab", { name: "main.tex", exact: true })).toHaveCount(0);
    await expect(warning).toHaveCount(0);
    await page.screenshot({ path: `.tmp/writing-list-feedback-${url.includes("preview") ? "preview" : "app"}.png` });
  });
}
