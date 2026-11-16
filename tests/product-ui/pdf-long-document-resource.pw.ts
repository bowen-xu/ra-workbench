import { existsSync } from "node:fs";
import {referenceProjectPath} from "../ui/projectPaths";

import { expect, test } from "@playwright/test";

import { openPdfAdapterFixture } from "../ui/pdfAdapterFixtureHarness";

const longDocumentFixture = referenceProjectPath(
  "zotero-reader/pdfjs/pdf.js/test/pdfs/freeculture.pdf",
);

test("bounds live canvases while navigating a frozen 352-page PDF", async ({ page }) => {
  test.skip(!existsSync(longDocumentFixture), "run pnpm reader:fixtures to prepare the frozen PDF set");
  const result = await openPdfAdapterFixture(page, longDocumentFixture, {
    navigatePageIndex: 200,
    measureResources: true,
  });

  expect(result.pageCount).toBe(352);
  expect(result.pageHostCount).toBe(352);
  expect(result.active?.resourceApiAvailable).toBe(true);
  expect(result.active?.cachedPageIds.length).toBeLessThanOrEqual(6);
  expect(result.active?.canvasCount).toBeLessThanOrEqual(6);
  expect(result.trimmed?.cachedPageIds.length).toBeLessThanOrEqual(4);
  expect(result.suspended).toMatchObject({ suspended: true, canvasCount: 0 });
});
