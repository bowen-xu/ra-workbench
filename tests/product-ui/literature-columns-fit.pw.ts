import { expect, test, type Page } from "@playwright/test";

async function metrics(page: Page) {
  return page.locator(".literature-table-scroll").evaluate((element) => {
    const headers = [...element.querySelectorAll<HTMLElement>(".literature-column")];
    const cells = [...element.querySelector(".literature-row")!.querySelectorAll<HTMLElement>(".literature-cell")];
    const bounds = headers.map((header) => header.getBoundingClientRect());
    return {
      width: element.clientWidth,
      scrollWidth: element.scrollWidth,
      columns: bounds.map((rect) => rect.width),
      lastRight: bounds.at(-1)?.right ?? 0,
      viewportRight: element.getBoundingClientRect().left + element.clientWidth,
      aligned: cells.every((cell, index) => {
        const rect = cell.getBoundingClientRect();
        const header = bounds[index];
        return header !== undefined && Math.abs(rect.left - header.left) < 1 && Math.abs(rect.width - header.width) < 1;
      }),
    };
  });
}

for (const entry of ["/?preview=app", "/?demo=app"]) {
  test(`columns shrink with the detail pane and restore their preferred widths at ${entry}`, async ({ page }) => {
    await page.setViewportSize({ width: 1515, height: 808 });
    await page.goto(entry);
    await expect(page.locator(".literature-row")).toHaveCount(5);
    await expect.poll(async () => (await metrics(page)).scrollWidth - (await metrics(page)).width).toBeLessThanOrEqual(1);
    const initial = await metrics(page);
    const separator = page.getByRole("separator", { name: "调整文献信息栏宽度", exact: true });
    const box = await separator.boundingBox();
    if (box === null) throw new Error("detail separator is missing");
    await page.mouse.move(box.x + box.width / 2, box.y + 80);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 - 100, box.y + 80, { steps: 8 });
    await page.mouse.up();
    await expect.poll(async () => (await metrics(page)).width).toBeLessThan(initial.width - 80);
    await expect.poll(async () => (await metrics(page)).scrollWidth - (await metrics(page)).width).toBeLessThanOrEqual(1);
    const shrunk = await metrics(page);
    shrunk.columns.forEach((width, index) => expect(width).toBeLessThan(initial.columns[index] ?? 0));
    expect(shrunk.aligned).toBe(true);
    expect(shrunk.lastRight).toBeLessThanOrEqual(shrunk.viewportRight + 1);
    if (entry === "/?demo=app") await page.screenshot({ path: ".tmp/literature-columns-fit.png" });

    await separator.dblclick();
    await expect.poll(async () => (await metrics(page)).columns).toEqual(initial.columns);
    await page.setViewportSize({ width: 1200, height: 808 });
    await expect.poll(async () => (await metrics(page)).width).toBeLessThan(484);
    const cramped = await metrics(page);
    expect(cramped.columns).toEqual([212, 90, 82, 58, 42]);
    expect(cramped.scrollWidth).toBeGreaterThan(cramped.width);
    await page.locator(".literature-table-scroll").evaluate((element) => { element.scrollLeft = element.scrollWidth; });
    expect((await metrics(page)).lastRight).toBeLessThanOrEqual((await metrics(page)).viewportRight + 1);
    await page.setViewportSize({ width: 1515, height: 808 });
    await expect.poll(async () => (await metrics(page)).columns).toEqual(initial.columns);
    await page.getByRole("button", { name: "切换关系栏", exact: true }).click();
    await expect.poll(async () => (await metrics(page)).columns[0] ?? 0).toBeGreaterThan(initial.columns[0] ?? 0);
    expect((await metrics(page)).aligned).toBe(true);
    const handles = page.locator(".literature-columns .column-resizer");
    await expect(handles).toHaveCount(4);
    await expect(page.locator(".literature-column:last-child .column-resizer")).toHaveCount(0);
    for (const [index, label] of ["附件", "年份", "条目类型", "作者"].entries()) {
      await page.getByRole("button", { name: "按标题排序", exact: true }).click({ button: "right" });
      await page.getByRole("menu", { name: "显示列", exact: true }).getByRole("menuitemcheckbox", { name: label, exact: true }).click();
      await expect(handles).toHaveCount(3 - index);
      await expect(page.locator(".literature-column:last-child .column-resizer")).toHaveCount(0);
    }
    await page.getByRole("button", { name: "按标题排序", exact: true }).click({ button: "right" });
    await page.getByRole("menu", { name: "显示列", exact: true }).getByRole("menuitemcheckbox", { name: "附件", exact: true }).click();
    await expect(handles).toHaveCount(1);
    await expect(page.getByRole("separator", { name: "调整标题列宽", exact: true })).toBeVisible();
    await expect(page.getByRole("separator", { name: "调整附件列宽", exact: true })).toHaveCount(0);
  });
}
