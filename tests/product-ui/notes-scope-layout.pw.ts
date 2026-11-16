import { expect, test } from "@playwright/test";

for (const url of ["/", "/?preview=app"]) {
  test(`note scope controls remain unclipped in a crowded sidebar on ${url}`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 600 });
    await page.goto(url);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    const explorer = page.locator(".notes-explorer");
    for (let index = 0; index < 6; index++) {
      await page.getByRole("tab", { name: "页面", exact: true }).click();
      const count = await explorer.getByRole("treeitem").count();
      await explorer.getByRole("button", { name: "新建页面", exact: true }).click();
      await expect(explorer.getByRole("treeitem")).toHaveCount(count + 1);
    }
    await page.getByRole("tab", { name: "全部笔记", exact: true }).click();
    const scopes = explorer.locator(".notes-scope-tabs");
    await expect.poll(() => scopes.evaluate(element => {
      const row = element.getBoundingClientRect();
      return Array.from(element.querySelectorAll("button")).every(button => {
        const bounds = button.getBoundingClientRect();
        return bounds.height >= 28 && bounds.top >= row.top && bounds.bottom <= row.bottom;
      });
    })).toBe(true);
    await expect(scopes.getByRole("button", { name: "全局", exact: true })).toBeVisible();
    await expect(scopes.getByRole("button", { name: "收藏", exact: true })).toBeVisible();
  });
}
