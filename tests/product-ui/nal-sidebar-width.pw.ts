import { expect, test } from "@playwright/test";

for (const route of ["/", "/?preview=app"]) {
  test(`concept sidebar resizes, remembers width, resets and fits narrow containers at ${route}`, async ({ page }) => {
    await page.setViewportSize({ width: 1272, height: 808 });
    await page.goto(route);
    const sidebar = page.getByRole("complementary", { name: "文献概念与关系", exact: true });
    const handle = page.getByRole("separator", { name: "调整概念与关系栏宽度", exact: true });
    const width = async () => (await sidebar.boundingBox())?.width ?? 0;
    await expect(handle).toBeVisible();
    const initial = await width();
    expect(initial).toBeGreaterThan(220);
    const bounds = await handle.boundingBox();
    if (!bounds) throw new Error("concept sidebar separator is missing");
    await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + 80);
    await page.mouse.down();
    await page.mouse.move(bounds.x + bounds.width / 2 + 100, bounds.y + 80, { steps: 6 });
    await page.mouse.up();
    await expect.poll(width).toBe(initial + 100);
    await page.reload();
    await expect.poll(width).toBe(initial + 100);
    await handle.press("ArrowRight");
    await expect.poll(width).toBe(initial + 110);
    await page.getByRole("button", { name: "切换关系栏", exact: true }).click();
    await expect(handle).toHaveCount(0);
    await page.getByRole("button", { name: "切换关系栏", exact: true }).click();
    await expect.poll(width).toBe(initial + 110);
    await page.setViewportSize({ width: 760, height: 640 });
    await expect(handle).toBeVisible();
    await expect.poll(width).toBeLessThan(initial);
    const containerWidth = await page.locator(".library-with-relations").evaluate(el => el.clientWidth);
    expect(await width()).toBeLessThanOrEqual(containerWidth / 2);
    await page.setViewportSize({ width: 1272, height: 808 });
    await expect.poll(width).toBe(initial + 110);
    await handle.dblclick();
    await expect.poll(width).toBe(initial);
    await page.reload();
    await expect.poll(width).toBe(initial);
    await handle.press("ArrowLeft");
    await expect.poll(width).toBe(initial - 10);
    await expect(handle).toHaveAttribute("aria-valuenow", String(initial - 10));
  });
}
