import { expect, test } from "@playwright/test";

for (const url of ["/", "/?preview=app"]) {
  test(`model management opens storage and reports and removes independent resources on ${url}`, async ({ page }) => {
    await page.goto(url);
    await page.getByRole("button", { name: "设置", exact: true }).click();
    await page.getByRole("button", { name: "本地服务", exact: true }).click();
    const manage = page.getByRole("button", { name: "管理模型存储", exact: true });
    await manage.scrollIntoViewIfNeeded();
    await page.mouse.move(1, 1);
    await manage.hover();
    await expect(page.locator(".control-hint:visible")).toContainText("管理模型存储");
    await manage.click();
    await expect(page.getByRole("heading", { name: "本地存储", exact: true })).toBeVisible();
    await expect(page.getByText("Docling FP32 模型", { exact: true })).toBeVisible();
    const remove = page.getByRole("button", { name: "删除模型", exact: true });
    if (!url.includes("preview")) { await expect(remove).toBeDisabled(); return; }
    await expect(page.getByText("Docling 模型", { exact: true })).toBeVisible();
    await expect(remove).toBeEnabled();
    const before = await page.locator(".settings-storage-total strong").textContent();
    await page.screenshot({ path: test.info().outputPath("model-storage.png"), animations: "disabled" });
    page.once("dialog", dialog => dialog.dismiss());
    await remove.click();
    await expect(remove).toBeEnabled();
    await expect(page.locator(".settings-storage-total strong")).toHaveText(before!);
    page.once("dialog", dialog => dialog.accept());
    await remove.click();
    await expect(remove).toBeDisabled();
    await expect(page.getByText(/0 B · 未安装可用模型/)).toBeVisible();
    await expect(page.locator(".settings-storage-total strong")).not.toHaveText(before!);
    await page.getByRole("button", { name: "本地服务", exact: true }).click();
    await expect(page.getByText("尚未安装。模型独立于软件安装包，修改更新源后检查并下载。", { exact: true })).toBeVisible({ timeout: 8000 });
  });
}
