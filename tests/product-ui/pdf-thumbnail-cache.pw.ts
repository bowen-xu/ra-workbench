import { expect, test, type Locator } from "@playwright/test";

async function expectBoundedVisibleBitmaps(thumbnails: Locator): Promise<void> {
  await expect.poll(async () => thumbnails.evaluate(list => {
    const navigation = list.closest(".reader-navigation");
    if (!navigation) return false;
    const bounds = navigation.getBoundingClientRect();
    const pages = [...list.querySelectorAll<HTMLElement>('[role="option"]')];
    const visible = pages.filter(page => {
      const rect = page.getBoundingClientRect();
      return rect.bottom > bounds.top && rect.top < bounds.bottom;
    });
    const allocated = pages.filter(page => (page.querySelector("canvas")?.width ?? 0) > 0);
    const completed = pages.filter(page => page.querySelector("canvas:not(.pending)") !== null);
    const budget = Math.min(8, visible.length * 2);
    return budget > 0 && allocated.length <= budget && completed.length <= budget
      && visible.every(page => completed.includes(page));
  }), { timeout: 15_000 }).toBe(true);
}

for (const entry of ["/?preview=app", "/?demo=app"]) {
  test(`thumbnail bitmap cache follows the viewport rather than accumulating: ${entry}`, async ({ page }) => {
    await page.setViewportSize({ width: 1434, height: 898 });
    await page.goto(entry);
    await page.getByRole("row", { name: /Vision by Logic/ }).dblclick();
    const reader = page.getByLabel("Vision by Logic: A Naïve Visual Recognition Model with Non-axiomatic Logic PDF 阅读器", { exact: true });
    const thumbnails = reader.getByRole("listbox", { name: "PDF 缩略图", exact: true });
    await expect(thumbnails.getByRole("option")).toHaveCount(15);
    await expectBoundedVisibleBitmaps(thumbnails);

    for (const pageNumber of [5, 10, 15, 1]) {
      await thumbnails.getByRole("option", { name: `第 ${pageNumber} 页`, exact: true }).scrollIntoViewIfNeeded();
      await expectBoundedVisibleBitmaps(thumbnails);
    }
    await page.setViewportSize({ width: 1434, height: 520 });
    await expectBoundedVisibleBitmaps(thumbnails);
    await expect(reader).not.toContainText("startCleanup");
  });
}
