import { expect, test } from "@playwright/test";

/** Known annotation rectangles on plain and rotated pages, independent of viewer geometry. */
function emailPdf(nativeLinks = true): Buffer {
  const content = "BT /F1 16 Tf 60 200 Td (reader@example.org) Tj ET";
  const objects = [
    "<< /Type /Catalog /Pages 2 0 R >>",
    "<< /Type /Pages /Count 2 /Kids [3 0 R 4 0 R] >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R /Annots [7 0 R] >>",
    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Rotate 90 /Resources << /Font << /F1 5 0 R >> >> /Contents 6 0 R /Annots [8 0 R] >>",
    "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
    ...Array.from({ length: 2 }, () => "<< /Type /Annot /Subtype /Link /Rect [60 195 230 215] /Border [0 0 0] /A << /S /URI /URI (mailto:reader@example.org) >> >>"),
  ];
  let pdf = "%PDF-1.7\n";
  const offsets = objects.map((object, index) => {
    const offset = pdf.length;
    pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
    return offset;
  });
  const xref = pdf.length;
  pdf += `xref\n0 9\n0000000000 65535 f \n${offsets.map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("")}trailer\n<< /Size 9 /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
  return Buffer.from(nativeLinks ? pdf : pdf.replace(/\/Annots \[\d 0 R\]/g, (value) => " ".repeat(value.length)));
}

for (const scale of [0.5, 1.375, 2.2]) {
  test(`email hover follows page content at ${scale} zoom, after scroll and rotation`, async ({ page }) => {
    await page.route("**/__links.pdf", (route) => route.fulfill({ body: emailPdf(), contentType: "application/pdf" }));
    await page.goto("/");
    await page.evaluate(async (scale) => {
      await import("/src/core/components/pdf-view/PdfView.css");
      const { PdfViewController } = await import("/src/core/components/pdf-view/PdfViewController.ts");
      const container = document.createElement("div");
      container.className = "pdf-view pdf-viewer-container";
      // Host placement and padding must not become the page coordinate system.
      Object.assign(container.style, { inset: "45px 70px 35px 110px", padding: "17px", border: "7px solid gray" });
      const viewer = document.createElement("div");
      viewer.className = "pdfViewer";
      container.append(viewer);
      document.body.replaceChildren(container);
      const controller = new PdfViewController();
      (window as any).linkController = controller;
      await controller.open({
        source: { key: "links", revision: "1", bytes: new Uint8Array(await fetch("/__links.pdf").then((r) => r.arrayBuffer())) },
        container, viewerElement: viewer, active: true, initialScale: { kind: "numeric", value: scale },
      });
    }, scale);

    for (const pageNumber of [1, 2]) {
      const sheet = page.locator(`.page[data-page-number="${pageNumber}"]`);
      await sheet.scrollIntoViewIfNeeded();
      const link = sheet.locator('.annotationLayer a[href="mailto:reader@example.org"]');
      await expect(link).toBeVisible();
      await link.hover();
      const geometry = await sheet.evaluate((element) => {
        const rect = (selector: string) => {
          const r = element.querySelector(selector)!.getBoundingClientRect();
          return { x: r.x, y: r.y, width: r.width, height: r.height };
        };
        return { canvas: rect("canvas"), annotation: rect(".annotationLayer"), text: rect(".textLayer"), link: rect(".annotationLayer a") };
      });
      // Raw /Rect mapped into the displayed page. Rotating 90 degrees swaps axes.
      const normalized = pageNumber === 1
        ? { x: 60 / 300, y: 185 / 400, width: 170 / 300, height: 20 / 400 }
        : { x: 195 / 400, y: 60 / 300, width: 20 / 400, height: 170 / 300 };
      for (const layer of [geometry.annotation, geometry.text]) {
        for (const key of ["x", "y", "width", "height"] as const) {
          expect(Math.abs(layer[key] - geometry.canvas[key])).toBeLessThan(1);
        }
      }
      const expected = {
        x: geometry.canvas.x + normalized.x * geometry.canvas.width,
        y: geometry.canvas.y + normalized.y * geometry.canvas.height,
        width: normalized.width * geometry.canvas.width,
        height: normalized.height * geometry.canvas.height,
      };
      for (const key of ["x", "y", "width", "height"] as const) {
        expect(Math.abs(geometry.link[key] - expected[key])).toBeLessThan(1);
      }
    }
    await page.evaluate(() => (window as any).linkController.close());
  });
}

test("automatic email links use glyphs even when DOM Range geometry is wrong", async ({ page }) => {
  await page.route("**/__automatic.pdf", (route) => route.fulfill({ body: emailPdf(false), contentType: "application/pdf" }));
  await page.goto("/");
  await page.evaluate(async () => {
    await import("/src/core/components/pdf-view/PdfView.css");
    const { PdfViewController } = await import("/src/core/components/pdf-view/PdfViewController.ts");
    // Reproduce an unusable text-layer measurement without changing painted PDF glyphs.
    Range.prototype.getClientRects = () => { throw new Error("Automatic links must not measure DOM ranges"); };
    const container = document.createElement("div"); container.className = "pdf-view pdf-viewer-container";
    const viewer = document.createElement("div"); viewer.className = "pdfViewer";
    container.append(viewer); document.body.replaceChildren(container);
    const controller = new PdfViewController(); (window as any).linkController = controller;
    await controller.open({ source: { key: "automatic", revision: "1", bytes: new Uint8Array(await fetch("/__automatic.pdf").then((r) => r.arrayBuffer())) },
      container, viewerElement: viewer, active: true, initialScale: { kind: "numeric", value: 1.375 } });
  });
  for (const pageNumber of [1, 2]) {
    const sheet = page.locator(`.page[data-page-number="${pageNumber}"]`);
    await sheet.scrollIntoViewIfNeeded();
    const link = sheet.locator('.annotationLayer a[href="mailto:reader@example.org"]');
    await expect(link).toHaveCount(1);
    await link.hover();
    const difference = await page.evaluate(async (pageNumber) => {
      const { beforePdfCharacter, afterPdfCharacter, createPdfCharacterSelection } = await import("/src/core/components/pdf-view/pdfPageCharacterSelection.ts");
      const c = (window as any).linkController;
      const model = c.adapter.characterStore.get(pageNumber - 1);
      const selection = createPdfCharacterSelection(model, beforePdfCharacter(0), afterPdfCharacter(model.chars.length - 1));
      const expected = c.adapter.clientRectForPageRect(pageNumber - 1, selection!.rects[0]);
      const actual = document.querySelector(`.page[data-page-number="${pageNumber}"] .annotationLayer a`)!.getBoundingClientRect();
      return ["x", "y", "width", "height"].map((key) => Math.abs((actual as any)[key] - expected[key]));
    }, pageNumber);
    expect(Math.max(...difference)).toBeLessThan(1);
  }
  await page.evaluate(() => (window as any).linkController.close());
  await expect(page.locator('.annotationLayer a')).toHaveCount(0);
});
