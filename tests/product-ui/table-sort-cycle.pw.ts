import { expect, test } from "@playwright/test";

for (const entry of ["/?preview=app", "/?demo=app"]) {
  test(`table headers restore default order after three clicks at ${entry}`, async ({ page }) => {
    await page.goto(entry);
    const rows = page.locator(".literature-row");
    await expect(rows).toHaveCount(5);
    const original = await rows.allTextContents();
    await rows.nth(1).click();
    const selected = await rows.nth(1).getAttribute("aria-label") ?? "";
    expect(selected).not.toBe("");
    for (const label of ["标题", "作者", "条目类型", "年份", "附件"]) {
      const header = page.getByRole("columnheader").filter({ has: page.getByRole("button", { name: `按${label}排序`, exact: true }) });
      const button = header.getByRole("button", { name: `按${label}排序`, exact: true });
      for (const state of ["ascending", "descending", "none"]) {
        await button.click();
        await expect(header).toHaveAttribute("aria-sort", state);
        await expect(page.getByRole("row", { name: selected, exact: true })).toHaveAttribute("aria-selected", "true");
      }
      expect(await rows.allTextContents()).toEqual(original);
      await expect(header.locator(".lucide-arrow-up, .lucide-arrow-down")).toHaveCount(0);
    }
    await page.getByRole("button", { name: "按标题排序", exact: true }).click();
    await page.getByRole("button", { name: "按年份排序", exact: true }).click();
    await expect(page.locator('.literature-column.title')).toHaveAttribute("aria-sort", "none");
    await expect(page.locator('.literature-column.year')).toHaveAttribute("aria-sort", "ascending");

    await page.getByRole("button", { name: "写作", exact: true }).click();
    const table = page.getByRole("table", { name: "稿件列表", exact: true });
    await expect(table).toBeVisible();
    const initial = await table.locator("tbody tr").allTextContents();
    for (const label of ["标题", "最近修改"]) {
      const header = table.getByRole("columnheader").filter({ has: page.getByRole("button", { name: label, exact: true }) });
      for (const state of ["ascending", "descending", "none"]) {
        await header.getByRole("button", { name: label, exact: true }).click();
        await expect(header).toHaveAttribute("aria-sort", state);
      }
      await expect(header.locator("svg")).toHaveCount(0);
      expect(await table.locator("tbody tr").allTextContents()).toEqual(initial);
    }
  });
}
