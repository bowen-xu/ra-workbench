import { expect, test, type Locator, type Page } from "@playwright/test";

async function swipeHide(page: Page, toast: Locator): Promise<void> {
  await toast.click({ trial: true, position: { x: 20, y: 30 } });
  const bounds = (await toast.boundingBox())!;
  await page.mouse.move(bounds.x + 20, bounds.y + bounds.height / 2);
  await page.mouse.down();
  await page.mouse.move(bounds.x + 160, bounds.y + bounds.height / 2, { steps: 10 });
  await page.mouse.up();
  await expect(toast).toHaveCount(0);
}

for (const route of ["/", "/?preview=app"]) {
  test(`Docling captions and icon actions share the production notification on ${route}`, async ({ page }) => {
    await page.goto(route);
    await expect(page.getByRole("button", { name: "设置", exact: true })).toBeVisible();
    await page.evaluate(async () => {
      const path = "/ra-workbench/apps/web/src/core/services/doclingParseTask.ts";
      const { runDoclingParse } = await import(path);
      let cancelled = false;
      const status = () => ({ id: "caption-task", state: cancelled ? "cancelled" : "running", stage: "parsing", completedPages: 8, totalPages: 15 });
      const gateway = { startParseTask: async () => status(), getParseTask: async () => status(), cancelParseTask: async () => { cancelled = true; return status(); } };
      void runDoclingParse(gateway, "paper", "pdf", {
        paperTitle: "Vision by Logic: A Naïve Visual Recognition Model with Non-axiomatic Logic — Synthetic Research Preview",
        fileName: "Xu et al. - 2026 - Vision by Logic A Naïve Visual Recognition Model with Non-axiomatic Logic.pdf",
      });
    });
    const toast = page.locator('[data-sonner-toast]').filter({ hasText: "正在解析 PDF" });
    await expect(toast).toBeVisible();
    const lines = toast.locator(".docling-notification-caption-line");
    await expect(lines).toHaveCount(2);
    await expect(lines.nth(0)).toHaveText(/^Vision by Logic.+\.\.\.$/);
    await expect(lines.nth(1)).toHaveText(/^Xu et al\..+\.\.\..+\.pdf$/);
    const sizes = await lines.evaluateAll(elements => elements.map(element => ({ width: element.clientWidth, height: element.clientHeight, lineHeight: Number.parseFloat(getComputedStyle(element).lineHeight) })));
    expect(sizes[0]!.width).toBe(220);
    expect(sizes[1]!.width).toBe(220);
    for (const size of sizes) expect(size.height).toBeLessThanOrEqual(Math.ceil(size.lineHeight));
    const stop = toast.getByRole("button", { name: "终止解析", exact: true });
    await expect(stop.locator("svg")).toHaveCount(1);
    await expect(stop.locator("svg")).toHaveClass(/lucide-square/);
    await expect(stop).toHaveText("");
    const iconLayout = await stop.evaluate(button => {
      const icon = button.querySelector("svg")!;
      const bounds = button.getBoundingClientRect();
      const iconBounds = icon.getBoundingClientRect();
      const shape = icon.querySelector("rect")!;
      const graphic = shape.getBBox();
      const graphicCenter = new DOMPoint(graphic.x + graphic.width / 2, graphic.y + graphic.height / 2).matrixTransform(shape.getScreenCTM()!);
      return {
        caption: Number.parseFloat(getComputedStyle(button).getPropertyValue("--ui-font-caption")),
        width: iconBounds.width, height: iconBounds.height,
        offsetX: iconBounds.x + iconBounds.width / 2 - bounds.x - bounds.width / 2,
        offsetY: iconBounds.y + iconBounds.height / 2 - bounds.y - bounds.height / 2,
        graphicOffsetX: graphicCenter.x - bounds.x - bounds.width / 2,
        graphicOffsetY: graphicCenter.y - bounds.y - bounds.height / 2,
      };
    });
    expect(iconLayout.width).toBeCloseTo(iconLayout.caption, 3);
    expect(iconLayout.height).toBeCloseTo(iconLayout.caption, 3);
    expect(iconLayout.offsetX, JSON.stringify(iconLayout)).toBeCloseTo(0, 3);
    expect(iconLayout.offsetY, JSON.stringify(iconLayout)).toBeCloseTo(0, 3);
    expect(iconLayout.graphicOffsetX, JSON.stringify(iconLayout)).toBeCloseTo(0, 3);
    expect(iconLayout.graphicOffsetY, JSON.stringify(iconLayout)).toBeCloseTo(0, 3);
    await stop.hover();
    const hint = page.getByRole("tooltip");
    await expect(hint).toContainText("终止解析");
    // A visible tooltip DOM node may still paint underneath Sonner's stack.
    // Hints ignore pointer input; temporarily enable it to test actual order.
    await hint.evaluate(element => { (element as HTMLElement).style.pointerEvents = "auto"; });
    await expect.poll(() => hint.evaluate(element => {
      const rect = element.getBoundingClientRect();
      const hit = document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2);
      return hit !== null && element.contains(hit);
    })).toBe(true);
    await hint.evaluate(element => { (element as HTMLElement).style.pointerEvents = "none"; });
    expect(await hint.evaluate(element => element.closest('[data-overlay-scope="notification"]') ===
      document.querySelector('[data-sonner-toaster]')?.closest('[data-overlay-scope="notification"]'))).toBe(true);
    await stop.screenshot({ path: test.info().outputPath("lucide-stop.png"), animations: "disabled" });
    await page.screenshot({ path: test.info().outputPath("docling-caption.png"), animations: "disabled" });
    await swipeHide(page, toast);
    const restore = page.getByRole("button", { name: "显示 1 条隐藏消息", exact: true });
    const toolbar = page.getByRole("banner", { name: "窗口工具栏" });
    const bellBounds = (await restore.boundingBox())!;
    const toolbarBounds = (await toolbar.boundingBox())!;
    expect(bellBounds.y).toBeGreaterThanOrEqual(toolbarBounds.y);
    expect(bellBounds.y + bellBounds.height).toBeLessThanOrEqual(toolbarBounds.y + toolbarBounds.height);
    expect(bellBounds.x + bellBounds.width).toBeGreaterThan(toolbarBounds.x + toolbarBounds.width - 20);
    await page.screenshot({ path: test.info().outputPath("notification-toolbar-bell.png"), animations: "disabled" });
    await restore.click();
    await stop.click();
    await expect(page.getByText("PDF 解析已终止", { exact: true })).toBeVisible();
    await expect(restore).toHaveCount(0);
  });
  test(`protected notifications hide, update, restore and settle on ${route}`, async ({ page }) => {
    await page.goto(route);
    await expect(page.getByRole("button", { name: "设置", exact: true })).toBeVisible();
    await page.evaluate(async () => {
      const path = "/ra-workbench/apps/web/src/core/services/notifications.ts";
      const { notify } = await import(path);
      notify.loading("解析测试 · 1/20", { id: "task-probe", description: "合成任务", actions: [{ id: "stop", label: "终止", onClick: async () => {
        notify.update("task-probe", { message: "正在终止合成任务…", actions: [{ id: "stop", label: "正在终止", onClick() {} }] });
        await new Promise<void>(resolve => { (window as unknown as { finishNotificationAction: () => void }).finishNotificationAction = resolve; });
      } }] });
    });
    const toast = page.locator('[data-sonner-toast]').filter({ hasText: "解析测试" });
    await expect(toast).toBeVisible();
    await expect(page.getByRole("button", { name: "隐藏通知", exact: true })).toHaveCount(0);
    await swipeHide(page, toast);
    const restore = page.getByRole("button", { name: "显示 1 条隐藏消息", exact: true });
    await expect(restore).toBeVisible();
    await expect(page.getByRole("banner", { name: "窗口工具栏" }).getByRole("button", { name: "显示 1 条隐藏消息" })).toBeVisible();
    expect(await restore.evaluate(element => getComputedStyle(element).position)).toBe("static");
    await expect(toast).toHaveCount(0);
    await page.evaluate(async () => { const path = "/ra-workbench/apps/web/src/core/services/notifications.ts"; (await import(path)).notify.update("task-probe", { message: "解析测试 · 7/20" }); });
    await expect(page.getByText("解析测试 · 7/20", { exact: true })).toBeHidden();
    await restore.hover();
    await expect(page.locator(".control-hint:visible")).toContainText("显示 1 条隐藏消息");
    await restore.click();
    await expect(page.getByText("解析测试 · 7/20", { exact: true })).toBeVisible();
    await page.getByRole("button", { name: "终止", exact: true }).click();
    await expect(page.getByText("正在终止合成任务…", { exact: true })).toBeVisible();
    await expect(page.getByRole("button", { name: "正在终止", exact: true })).toBeDisabled();
    await swipeHide(page, page.locator('[data-sonner-toast]').filter({ hasText: "正在终止合成任务" }));
    await expect(restore).toBeVisible();
    await expect(page.getByText("正在终止合成任务…", { exact: true })).toHaveCount(0);
    await restore.click();
    await expect(page.getByRole("button", { name: "正在终止", exact: true })).toBeDisabled();
    await swipeHide(page, page.locator('[data-sonner-toast]').filter({ hasText: "正在终止合成任务" }));
    await expect(restore).toBeVisible();
    await page.evaluate(() => (window as unknown as { finishNotificationAction: () => void }).finishNotificationAction());
    await page.evaluate(async () => { const path = "/ra-workbench/apps/web/src/core/services/notifications.ts"; (await import(path)).notify.info("合成任务已终止", { id: "task-probe" }); });
    await expect(page.getByText("合成任务已终止", { exact: true })).toBeVisible();
    await expect(restore).toHaveCount(0);
    await expect(page.getByRole("button", { name: "正在终止", exact: true })).toHaveCount(0);
    await page.screenshot({ path: test.info().outputPath("task-notification.png"), animations: "disabled" });
  });
}
