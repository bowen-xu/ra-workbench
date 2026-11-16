import { existsSync } from "node:fs";
import {referenceProjectPath} from "./projectPaths";

import { expect, test } from "@playwright/test";

const twoColumnFixture = referenceProjectPath(
  "zotero-reader/pdfjs/pdf.js/test/pdfs/tracemonkey.pdf",
);

interface RealPdfSelectionProbe {
  readonly contentBounds: { readonly left: number; readonly width: number };
  readonly outerBounds: { readonly left: number; readonly width: number };
  readonly expectedLeft: number;
  readonly expectedRight: number;
  readonly adapterLeft: number;
  readonly adapterRight: number;
  readonly highlightLeft: number;
  readonly highlightRight: number;
  readonly outerBoxLeft: number;
  readonly outerBoxRight: number;
  readonly selectedText: string;
  readonly selectionRectCount: number;
}

test("keeps structured selection endpoints on real PDF glyphs at a non-integer zoom", async ({ page }) => {
  test.skip(!existsSync(twoColumnFixture), "run pnpm reader:fixtures to prepare the frozen PDF set");
  await page.route("**/__reader-fixtures/document.pdf", async (route) => {
    await route.fulfill({ path: twoColumnFixture, contentType: "application/pdf" });
  });
  await page.goto("/");

  const probe = await page.evaluate(async (): Promise<RealPdfSelectionProbe> => {
    await import("/src/core/components/pdf-view/PdfView.css");
    const { PdfViewAdapter } = await import("/src/core/components/pdf-view/PdfViewAdapter.ts");
    const {
      afterPdfCharacter,
      beforePdfCharacter,
      createPdfCharacterSelection,
    } = await import("/src/core/components/pdf-view/pdfPageCharacterSelection.ts");
    const { pdfPageContentRect } = await import("/src/core/components/pdf-view/pdfPageCharacterModel.ts");
    const bytes = new Uint8Array(await fetch("/__reader-fixtures/document.pdf")
      .then((response) => response.arrayBuffer()));
    const container = document.createElement("div");
    Object.assign(container.style, {
      position: "fixed",
      inset: "0",
      overflow: "auto",
      width: "980px",
      height: "760px",
    });
    const viewer = document.createElement("div");
    viewer.className = "pdfViewer";
    container.className = "pdf-view pdf-viewer-container";
    container.append(viewer);
    document.body.replaceChildren(container);

    let ready: (() => void) | undefined;
    let lastStatus = "";
    let characterSelectionAvailable = true;
    const readyPromise = new Promise<void>((resolveReady) => { ready = resolveReady; });
    const adapter = new PdfViewAdapter();
    await adapter.open({
      bytes,
      container,
      viewerElement: viewer,
      callbacks: {
        onReady: () => ready?.(),
        onPageChange: () => undefined,
        onScaleChange: () => undefined,
        onViewAreaChange: () => undefined,
        onPageHostsChange: () => undefined,
        onFindMatchesChange: () => undefined,
        onOutlineChange: () => undefined,
        onCharacterSelectionAvailabilityChange: (available: boolean) => { characterSelectionAvailable = available; },
        onResourceChange: () => undefined,
        onStatus: (status: string) => { lastStatus = status; },
      },
    });
    await readyPromise;
    adapter.navigate({ pageIndex: 0, scale: { kind: "numeric", value: 1.37 }, history: "hard" }, 0);

    const deadline = performance.now() + 8_000;
    let model = adapter.characterStore.get(0);
    let pageElement = viewer.querySelector<HTMLElement>('.page[data-page-number="1"]');
    while ((model === undefined || pageElement?.querySelector(":scope > .canvasWrapper") === null)
      && performance.now() < deadline) {
      await new Promise((resolveDelay) => window.setTimeout(resolveDelay, 50));
      pageElement = viewer.querySelector<HTMLElement>('.page[data-page-number="1"]');
      if (pageElement !== null) model = adapter.characterStore.syncPage(pageElement);
    }
    if (model === undefined || pageElement === null) {
      const diagnostic = JSON.stringify({
        characterSelectionAvailable,
        hasCanvas: pageElement?.querySelector(":scope > .canvasWrapper") !== null,
        hasPage: pageElement !== null,
        hasSource: adapter.characterStore.hasPageCharacters(0),
        lastStatus,
      });
      adapter.close();
      throw new Error(`real PDF character model did not become ready: ${diagnostic}`);
    }

    const candidate = model.lines
      .map((line) => line.characterIndexes.filter((index) => !/\s/u.test(model.chars[index]?.value ?? "")))
      .find((indexes) => {
        const first = model.chars[indexes[0] ?? -1];
        const last = model.chars[indexes.at(-1) ?? -1];
        return indexes.length >= 12
          && first !== undefined
          && last !== undefined
          && first.inlineRect.x < 0.25
          && last.inlineRect.x + last.inlineRect.width < 0.52;
      });
    if (candidate === undefined) {
      adapter.close();
      throw new Error("two-column fixture did not expose a selectable left-column line");
    }
    const selectedIndexes = candidate.slice(2, 10);
    const firstIndex = selectedIndexes[0];
    const lastIndex = selectedIndexes.at(-1);
    if (firstIndex === undefined || lastIndex === undefined) {
      adapter.close();
      throw new Error("real PDF selection range is empty");
    }
    const selection = createPdfCharacterSelection(
      model,
      beforePdfCharacter(firstIndex),
      afterPdfCharacter(lastIndex),
    );
    if (selection === undefined) {
      adapter.close();
      throw new Error("real PDF selection was not created");
    }
    const layer = pageElement.querySelector<HTMLElement>(":scope > .pdf-view-overlay-layer");
    if (layer === null) {
      adapter.close();
      throw new Error("real PDF highlight layer is missing");
    }
    layer.replaceChildren(...selection.rects.map((rect) => {
      const highlight = document.createElement("span");
      highlight.className = "pending-selection-highlight";
      Object.assign(highlight.style, {
        left: `${rect.x * 100}%`,
        top: `${rect.y * 100}%`,
        width: `${rect.width * 100}%`,
        height: `${rect.height * 100}%`,
      });
      return highlight;
    }));
    await new Promise<void>((resolveFrame) => window.requestAnimationFrame(() => resolveFrame()));

    const selectionRect = selection.rects.at(-1);
    const renderedRect = layer.querySelector<HTMLElement>(".pending-selection-highlight:last-child")
      ?.getBoundingClientRect();
    if (selectionRect === undefined || renderedRect === undefined) {
      adapter.close();
      throw new Error("real PDF selection rectangle was not rendered");
    }
    const contentBounds = pdfPageContentRect(pageElement);
    const outerBounds = pageElement.getBoundingClientRect();
    const expectedLeft = contentBounds.left + selectionRect.x * contentBounds.width;
    const expectedRight = contentBounds.left
      + (selectionRect.x + selectionRect.width) * contentBounds.width;
    const adapterRect = adapter.clientRectForPageRect(0, selectionRect);
    if (adapterRect === undefined) {
      adapter.close();
      throw new Error("real PDF adapter could not project a page rectangle");
    }
    const outerBoxLeft = outerBounds.left + selectionRect.x * outerBounds.width;
    const outerBoxRight = outerBounds.left
      + (selectionRect.x + selectionRect.width) * outerBounds.width;
    const result: RealPdfSelectionProbe = {
      contentBounds: { left: contentBounds.left, width: contentBounds.width },
      outerBounds: { left: outerBounds.left, width: outerBounds.width },
      expectedLeft,
      expectedRight,
      adapterLeft: adapterRect.left,
      adapterRight: adapterRect.right,
      highlightLeft: renderedRect.left,
      highlightRight: renderedRect.right,
      outerBoxLeft,
      outerBoxRight,
      selectedText: selection.text,
      selectionRectCount: selection.rects.length,
    };
    // Let PDF.js finish the adjacent visible-page text task before destroying
    // the worker so the regression output stays free of termination warnings.
    await new Promise((resolveDelay) => window.setTimeout(resolveDelay, 500));
    adapter.close();
    return result;
  });

  expect(probe.selectedText.length).toBeGreaterThanOrEqual(8);
  expect(probe.selectionRectCount).toBe(1);
  expect(probe.outerBounds.width - probe.contentBounds.width).toBeGreaterThan(10);
  expect(probe.adapterLeft).toBeCloseTo(probe.expectedLeft, 0);
  expect(probe.adapterRight).toBeCloseTo(probe.expectedRight, 0);
  expect(probe.highlightLeft).toBeCloseTo(probe.expectedLeft, 0);
  expect(probe.highlightRight).toBeCloseTo(probe.expectedRight, 0);
  expect(Math.max(
    Math.abs(probe.outerBoxLeft - probe.expectedLeft),
    Math.abs(probe.outerBoxRight - probe.expectedRight),
  )).toBeGreaterThan(2);
});
