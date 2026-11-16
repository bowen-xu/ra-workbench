import { expect, test } from "@playwright/test";
import { navigationPdf } from "./sharedPdfFixture";

test("isolates viewers, follows internal links and transforms a rotated page anchor", async ({ page }) => {
  const errors: string[] = []; page.on("pageerror", (e) => errors.push(e.message));
  await page.route("**/__navigation.pdf", (route) => route.fulfill({ body: navigationPdf(), contentType: "application/pdf" }));
  await page.goto("/");
  await page.evaluate(async () => {
    await import("/src/core/components/pdf-view/PdfView.css");
    const { PdfViewController } = await import("/src/core/components/pdf-view/PdfViewController.ts");
    const bytes = new Uint8Array(await fetch("/__navigation.pdf").then((r) => r.arrayBuffer()));
    const controllers = [];
    document.body.replaceChildren();
    for (let i = 0; i < 2; i++) {
      const container = document.createElement("div"); container.className = "pdf-view pdf-viewer-container";
      Object.assign(container.style, { position: "fixed", top: "0", bottom: "0", left: `${i * 620}px`, width: "600px", right: "auto" });
      const viewer = document.createElement("div"); viewer.className = "pdfViewer"; container.append(viewer); document.body.append(container);
      const controller = new PdfViewController(); controllers.push(controller);
      await controller.open({ source: { key: String(i), revision: "one", bytes }, container, viewerElement: viewer, active: true, initialScale: { kind: "numeric", value: 1.375 } });
    }
    (window as any).sharedControllers = controllers;
  });
  const left = page.locator(".pdf-view").first();
  await expect(left.locator(".annotationLayer a")).toBeVisible();
  await left.locator(".annotationLayer a").click();
  await expect.poll(() => page.evaluate(() => (window as any).sharedControllers[0].getSnapshot().pageIndex)).toBe(1);
  expect(await page.evaluate(() => (window as any).sharedControllers[1].getSnapshot().pageIndex)).toBe(0);
  await expect(left.locator('.page[data-page-number="2"] canvas').first()).toBeVisible();
  const probe = await page.evaluate(async () => {
    const c = (window as any).sharedControllers[0];
    const point = c.adapter.readingPoint();
    const rect = await c.adapter.normalizedPdfRect(point.pageIndex, { x: point.x, y: point.y, width: 0, height: 0 });
    const client = c.adapter.clientRectForPageRect(point.pageIndex, rect);
    const stage = document.querySelector(".pdf-view")!.getBoundingClientRect();
    return { pageIndex: point.pageIndex, x: client.x, y: client.y, top: stage.top, scale: c.getSnapshot().scalePercent };
  });
  expect(probe.pageIndex).toBe(1); expect(probe.scale).toBe(138);
  expect(probe.y - probe.top).toBeCloseTo(72, 0);
  expect(probe.x).toBeGreaterThan(100); expect(probe.x).toBeLessThan(500);
  await page.evaluate(() => (window as any).sharedControllers.forEach((c: any) => c.close()));
  await expect(page.locator(".pdfViewer .page")).toHaveCount(0);
  await page.setViewportSize({ width: 1100, height: 700 });
  expect(errors).toEqual([]);
});
