import { existsSync } from "node:fs";
import {referenceProjectPath} from "../ui/projectPaths";

import { expect, test } from "@playwright/test";

import { openPdfAdapterFixture } from "../ui/pdfAdapterFixtureHarness";

const mixedPageFixture = referenceProjectPath(
  "zotero-reader/pdfjs/pdf.js/test/pdfs/sizes.pdf",
);

test("restores the page and auto-fits a frozen mixed-page-size PDF", async ({ page }) => {
  test.skip(!existsSync(mixedPageFixture), "run pnpm reader:fixtures to prepare the frozen PDF set");
  const restoredViewState = {
    schemaVersion: 2 as const,
    pageIndex: 1,
    // Reproduces the historical percentage-as-ratio corruption seen as 19775%.
    scale: { kind: "numeric" as const, value: 197.75 },
    top: 120,
    left: 18,
    scrollMode: "vertical" as const,
    spreadMode: "none" as const,
  };
  const result = await openPdfAdapterFixture(page, mixedPageFixture, { restoredViewState });

  expect(result.pageCount).toBe(3);
  expect(result.readyViewState).toEqual({
    pageIndex: 1,
    scale: { kind: "page-fit" },
    scrollMode: "vertical",
    spreadMode: "none",
  });
  expect(result.pageChanges).toContain(1);
  expect(result.viewStates.some((state) => (
    state.pageIndex === 1
    && state.scale.kind === "page-fit"
  ))).toBe(true);
  const restoredCanvas = result.canvases.find((canvas) => canvas.pageNumber === 2 && canvas.intersectsViewport);
  expect(restoredCanvas, "restored page must have a visible PDF.js canvas").toBeDefined();
  expect(restoredCanvas?.bitmapWidth).toBeGreaterThan(0);
  expect(restoredCanvas?.bitmapHeight).toBeGreaterThan(0);
  expect(restoredCanvas?.cssWidth).toBeGreaterThan(0);
  expect(restoredCanvas?.cssHeight).toBeGreaterThan(0);
  expect(restoredCanvas?.hasPaintedPixels, "restored PDF canvas must contain rendered document pixels").toBe(true);
});
