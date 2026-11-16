import { expect, test } from "@playwright/test";
import { navigationPdf } from "../ui/sharedPdfFixture";

for (const route of ["/?preview=app", "/?demo=app"]) test(`PDF fit width has no horizontal overflow and wheel zoom preserves the pointer anchor: ${route}`, async ({ page }) => {
  await page.route("**/preview/vision-by-logic.pdf", request => request.fulfill({ body: navigationPdf(), contentType: "application/pdf" }));
  await page.goto(route);
  await page.getByRole("row", { name: /Vision by Logic/ }).dblclick();
  const reader = page.locator(".reader-workspace:visible");
  const container = reader.locator(".pdf-viewer-container");
  const paper = container.locator('.page[data-page-number="1"]');
  await expect(paper.locator("canvas").first()).toBeVisible();
  await expect(container).toHaveCSS("overscroll-behavior-x", "none");

  for (const viewport of [{ width: 1434, height: 898 }, { width: 1276, height: 808 }]) {
    await page.setViewportSize(viewport);
    await reader.getByRole("button", { name: "适合宽度", exact: true }).click();
    await expect.poll(() => container.evaluate(element => element.scrollWidth - element.clientWidth)).toBe(0);
    expect(await container.evaluate(element => { element.scrollLeft = 100; return element.scrollLeft; })).toBe(0);
  }

  for (let index = 0; index < 7; index += 1) await reader.getByRole("button", { name: "放大", exact: true }).click();
  await expect.poll(() => container.evaluate(element => element.scrollWidth - element.clientWidth)).toBeGreaterThan(150);
  await container.evaluate(element => { element.scrollLeft = 100; element.scrollTop = 180; });
  // Let PDF.js update its ordinary scroll location before testing a wheel gesture.
  await expect.poll(() => container.evaluate(element => element.scrollTop)).toBe(180);
  const anchor = await container.evaluate(element => {
    const bounds = element.getBoundingClientRect();
    const paper = element.querySelector<HTMLElement>('.page[data-page-number="1"]');
    if (!paper) throw new Error("Missing PDF page");
    const rect = paper.getBoundingClientRect(), style = getComputedStyle(paper);
    const x = bounds.left + element.clientWidth * .65, y = bounds.top + element.clientHeight * .45;
    return { x, y, width: rect.width,
      pageX: (x - rect.left - parseFloat(style.borderLeftWidth)) / parseFloat(style.width),
      pageY: (y - rect.top - parseFloat(style.borderTopWidth)) / parseFloat(style.height) };
  });
  await container.dispatchEvent("wheel", { ctrlKey: true, deltaY: -100, clientX: anchor.x, clientY: anchor.y });
  await expect.poll(() => paper.evaluate(element => element.getBoundingClientRect().width)).toBeGreaterThan(anchor.width);
  await expect.poll(() => paper.evaluate((element, point) => {
    const rect = element.getBoundingClientRect(), style = getComputedStyle(element);
    return Math.hypot(rect.left + parseFloat(style.borderLeftWidth) + point.pageX * parseFloat(style.width) - point.x,
      rect.top + parseFloat(style.borderTopWidth) + point.pageY * parseFloat(style.height) - point.y);
  }, anchor)).toBeLessThan(4);
  await page.screenshot({ path: `.tmp/verification/pdf-zoom-anchor-${route.includes("demo") ? "demo" : "preview"}.png` });
});
