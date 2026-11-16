import { expect, test } from "@playwright/test";

for (const route of ["/?preview=app", "/?demo=app"]) test(`literature drop feedback follows rows and detail selection: ${route}`, async ({ page }) => {
  await page.goto(route);
  const rows = page.locator(".literature-row");
  await expect(rows.first()).toBeVisible();
  const count = await rows.count();
  const first = rows.filter({ hasText: "Vision by Logic" });
  const second = rows.filter({ hasText: "Synthetic Research Preview" });
  await first.click();
  const detail = page.locator(".literature-detail");
  const surface = page.locator(".library-grid");

  const files = await page.evaluateHandle(() => {
    const transfer = new DataTransfer();
    transfer.items.add(new File(["%PDF-1.7\n"], "dropped.pdf", { type: "application/pdf" }));
    return transfer;
  });
  const hint = page.getByRole("status").filter({ hasText: "松开以" });
  for (const viewport of [{ width: 1434, height: 898 }, { width: 1276, height: 808 }]) {
    await page.setViewportSize(viewport);
    await second.locator(".literature-cell.title").dispatchEvent("dragover", { dataTransfer: files });
    await expect(hint).toContainText("Synthetic Research Preview");
    await expect(second.locator("..")).toHaveAttribute("data-file-drop-target", "true");
    await expect(first).toHaveAttribute("aria-selected", "true");
    const box = await hint.boundingBox();
    expect(box).not.toBeNull();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(viewport.width);
    expect(await second.evaluate(element => {
      const rect = element.getBoundingClientRect();
      return element.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
    })).toBe(true);
  }
  await page.screenshot({ path: `.tmp/verification/literature-drop-row-${route.includes("demo") ? "demo" : "preview"}.png` });
  await detail.dispatchEvent("dragover", { dataTransfer: files });
  await expect(detail).toHaveAttribute("data-file-drop-target", "true");
  await expect(surface).not.toHaveAttribute("data-file-drop-target", "true");
  await expect(hint).toContainText("Vision by Logic");
  await page.screenshot({ path: `.tmp/verification/literature-drop-detail-${route.includes("demo") ? "demo" : "preview"}.png` });

  for (const target of [page.locator(".literature-columns"), page.locator(".literature-list")]) {
    await target.dispatchEvent("dragover", { dataTransfer: files });
    await expect(hint).toContainText("独立文献");
    await expect(surface).toHaveAttribute("data-file-drop-target", "true");
    await expect(page.locator('.literature-list[data-file-drop-target="true"], .literature-detail[data-file-drop-target="true"], .literature-item-group[data-file-drop-target="true"]')).toHaveCount(0);
    expect(await surface.evaluate(element => {
      const skin = getComputedStyle(element, "::after");
      const list = element.querySelector(".literature-list");
      return { border: skin.borderStyle, pointer: skin.pointerEvents, position: skin.position,
        headerScope: list ? getComputedStyle(list).isolation : "" };
    })).toEqual({ border: "dashed", pointer: "none", position: "absolute", headerScope: "isolate" });
  }
  await page.screenshot({ path: `.tmp/verification/literature-drop-standalone-list-${route.includes("demo") ? "demo" : "preview"}.png` });
  await first.click({ modifiers: ["ControlOrMeta"] });
  await expect(detail.getByText("选择一篇文献查看详情", { exact: true })).toBeVisible();
  await detail.dispatchEvent("dragover", { dataTransfer: files });
  await expect(hint).toContainText("独立文献");
  await expect(surface).toHaveAttribute("data-file-drop-target", "true");
  await expect(detail).not.toHaveAttribute("data-file-drop-target", "true");
  expect(await detail.evaluate(element => {
    const rect = element.getBoundingClientRect();
    return element.contains(document.elementFromPoint(rect.x + rect.width / 2, rect.y + rect.height / 2));
  })).toBe(true);
  await page.screenshot({ path: `.tmp/verification/literature-drop-standalone-empty-detail-${route.includes("demo") ? "demo" : "preview"}.png` });
  await detail.dispatchEvent("drop", { dataTransfer: files });
  await expect(hint).toHaveCount(0);
  await expect(page.getByText("请在 Tauri 桌面模式中拖入 PDF 文件；浏览器无法读取本机文件路径。", { exact: true })).toBeVisible();
  await expect(rows).toHaveCount(count);
  await files.dispose();
});
