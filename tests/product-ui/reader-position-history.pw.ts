import { expect, test } from "@playwright/test";
import { navigationPdf } from "../ui/sharedPdfFixture";

for (const route of ["/?preview=app", "/?demo=app"]) {
  test(`reader toolbar returns from PDF links and preserves forward history on ${route}`, async ({ page }) => {
    const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
    await page.route("**/preview/vision-by-logic.pdf", request => request.fulfill({ body: navigationPdf(), contentType: "application/pdf" }));
    await page.goto(route);
    await page.getByRole("row", {name: /Vision by Logic/}).dblclick();
    const reader = page.locator(".reader-workspace:visible");
    const back = reader.getByRole("button", {name: "返回上一阅读位置", exact: true});
    const forward = reader.getByRole("button", {name: "前进到下一阅读位置", exact: true});
    const pageNumber = reader.getByRole("textbox", {name: "页码", exact: true});
    const stage = reader.locator(".pdf-viewer-container");
    const link = reader.locator('.page[data-page-number="1"] .annotationLayer a');
    await expect(link).toBeVisible();
    await expect(back).toBeDisabled(); await expect(forward).toBeDisabled();
    await stage.evaluate(element => { element.scrollTop = 30; });
    await expect.poll(() => stage.evaluate(element => element.scrollTop)).toBeCloseTo(30, 0);
    const origin = await stage.evaluate(element => element.scrollTop);
    await link.click();
    await expect(pageNumber).toHaveValue("2"); await expect(back).toBeEnabled();
    await back.click();
    await expect(pageNumber).toHaveValue("1");
    await expect.poll(async () => Math.abs(await stage.evaluate(element => element.scrollTop) - origin)).toBeLessThan(4);
    await expect(forward).toBeEnabled();
    await forward.click(); await expect(pageNumber).toHaveValue("2");
    await expect(forward).toBeDisabled();
    await back.click(); await expect(pageNumber).toHaveValue("1");
    // A new explicit jump after going back replaces the forward branch.
    await pageNumber.fill("2"); await pageNumber.press("Enter");
    await expect(pageNumber).toHaveValue("2"); await expect(forward).toBeDisabled();
    // Annotation undo is a separate command and remains unavailable without edits.
    await expect(reader.getByRole("button", {name: "撤销批注更改", exact: true})).toBeDisabled();
    await page.screenshot({path: `.tmp/reader-position-history-${route.includes("preview") ? "preview" : "app"}.png`});
    expect(errors).toEqual([]);
  });
}
