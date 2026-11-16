import { expect, test, type Page } from "@playwright/test";

interface SelectionProbe {
  readonly completed: number;
  readonly modelVersion: number;
  readonly nativeRangeCount: number;
  readonly nativeText: string;
  readonly rects: ReadonlyArray<{ readonly x: number; readonly y: number; readonly width: number; readonly height: number }>;
  readonly selecting: boolean;
}

async function installSelectionHarness(page: Page): Promise<void> {
  await page.goto("/");
  await page.evaluate(async () => {
    await import("/src/core/components/pdf-view/PdfView.css");
    const modelPath = "/src/core/components/pdf-view/pdfPageCharacterModel.ts";
    const controllerPath = "/src/core/components/pdf-view/pdfCharacterSelectionController.ts";
    const { PdfPageCharacterStore } = await import(modelPath);
    const { PdfCharacterSelectionController } = await import(controllerPath);
    const host = document.createElement("div");
    host.id = "pdf-selection-harness";
    host.className = "app-shell pdf-view pdf-viewer-container";
    Object.assign(host.style, {
      position: "fixed",
      inset: "0",
      zIndex: "10000",
      display: "block",
      background: "#d9dcda",
    });
    const viewer = document.createElement("div");
    viewer.className = "pdfViewer";
    const pdfPage = document.createElement("div");
    pdfPage.className = "page";
    pdfPage.dataset.pageNumber = "1";
    Object.assign(pdfPage.style, {
      position: "absolute",
      left: "80px",
      top: "40px",
      width: "720px",
      height: "480px",
      background: "white",
      border: "0", // Synthetic glyphs use the full page box; real PDF border geometry is tested separately.
    });
    const textLayer = document.createElement("div");
    textLayer.className = "textLayer";
    Object.assign(textLayer.style, { position: "absolute", inset: "0" });
    const addLine = (text: string, left: number, top: number, column: string): HTMLSpanElement => {
      const span = document.createElement("span");
      span.dataset.column = column;
      span.textContent = text;
      Object.assign(span.style, {
        position: "absolute",
        left: `${left}px`,
        top: `${top}px`,
        font: "20px/1.05 Georgia, serif",
        whiteSpace: "pre",
      });
      textLayer.append(span);
      return span;
    };
    addLine("Left column alpha beta", 40, 55, "left");
    addLine("Left column second line", 40, 88, "left");
    addLine("Right column must stay separate", 400, 55, "right");
    const highlightLayer = document.createElement("div");
    highlightLayer.className = "pdf-view-overlay-layer";
    pdfPage.append(textLayer, highlightLayer);
    viewer.append(pdfPage);
    host.append(viewer);
    document.body.append(host);

    const store = new PdfPageCharacterStore();
    const pageRect = pdfPage.getBoundingClientRect();
    const sourceCharacters = [...textLayer.querySelectorAll<HTMLElement>("span")].flatMap((span) => {
      const rect = span.getBoundingClientRect();
      const values = Array.from(span.textContent ?? "");
      const characterWidth = rect.width / values.length;
      return values.map((value, index) => ({
        value,
        rect: {
          x: (rect.left - pageRect.left + index * characterWidth) / pageRect.width,
          y: (rect.top - pageRect.top) / pageRect.height,
          width: characterWidth / pageRect.width,
          height: rect.height / pageRect.height,
        },
        rotation: 0,
        lineBreakAfter: index === values.length - 1,
      }));
    });
    store.setPageCharacters(0, sourceCharacters);
    type HarnessSelection = { segments: readonly { pageIndex: number; text?: string; rects: readonly { x: number; y: number; width: number; height: number }[] }[] };
    let completedSelection: HarnessSelection | undefined;
    let liveSelection: HarnessSelection | undefined;
    let completed = 0;
    const render = (): void => {
      const selection = liveSelection ?? completedSelection;
      for (const pageElement of viewer.querySelectorAll<HTMLElement>(".page[data-page-number]")) {
        const pageIndex = Number(pageElement.dataset.pageNumber) - 1;
        const layer = pageElement.querySelector<HTMLElement>(".pdf-view-overlay-layer");
        const rects = selection?.segments.find((segment) => segment.pageIndex === pageIndex)?.rects ?? [];
        layer?.replaceChildren(...rects.map((rect) => {
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
      }
    };
    const controller = new PdfCharacterSelectionController({
      viewer,
      pageStore: store,
      onSelectionStart: () => undefined,
      onSelectionChange: (selection: typeof liveSelection) => { liveSelection = selection; render(); },
      onSelectionComplete: (selection: typeof completedSelection) => {
        completed += 1;
        completedSelection = selection;
        render();
      },
    });
    Object.assign(window, {
      __pdfSelectionHarness: {
        controller,
        store,
        page: pdfPage,
        textLayer,
        getCompleted: () => completed,
        getSelection: () => completedSelection,
      },
    });
  });
}

async function probe(page: Page): Promise<SelectionProbe> {
  return page.evaluate(() => {
    const pdfPage = document.querySelector<HTMLElement>("#pdf-selection-harness .page");
    if (pdfPage === null) throw new Error("selection harness page is missing");
    const selection = window.getSelection();
    return {
      completed: (window as typeof window & { __pdfSelectionHarness: { getCompleted(): number } }).__pdfSelectionHarness.getCompleted(),
      modelVersion: Number(pdfPage.dataset.pdfCharacterModelVersion),
      nativeRangeCount: selection?.rangeCount ?? 0,
      nativeText: selection?.toString() ?? "",
      rects: [...pdfPage.querySelectorAll<HTMLElement>(".pending-selection-highlight")].map((element) => {
        const rect = element.getBoundingClientRect();
        return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
      }),
      selecting: pdfPage.dataset.customSelectionActive === "true",
    };
  });
}

test("uses one custom character selection layer before and after pointer release", async ({ page }) => {
  await installSelectionHarness(page);
  const lines = page.locator('#pdf-selection-harness [data-column="left"]');
  const first = await lines.nth(0).boundingBox();
  const second = await lines.nth(1).boundingBox();
  if (first === null || second === null) throw new Error("selection lines are not laid out");

  await page.mouse.move(first.x + 4, first.y + first.height / 2);
  await page.mouse.down();
  await page.mouse.move(second.x + second.width * 0.7, second.y + second.height / 2, { steps: 8 });
  await page.waitForTimeout(20);
  const during = await probe(page);
  expect(during.selecting).toBe(true);
  expect(during.completed).toBe(0);
  expect(during.rects.length).toBeGreaterThanOrEqual(2);
  expect(during.nativeText).toBe("");
  expect(during.nativeRangeCount).toBe(0);
  expect(during.rects[0]?.y).toBeCloseTo(first.y, 0);
  expect(during.rects[0]?.height).toBeCloseTo(first.height, 0);
  expect(during.rects[1]?.y).toBeCloseTo(second.y, 0);
  expect(during.rects[1]?.height).toBeCloseTo(second.height, 0);

  await page.mouse.up();
  const after = await probe(page);
  expect(after.selecting).toBe(false);
  expect(after.completed).toBe(1);
  expect(after.rects).toHaveLength(during.rects.length);
  after.rects.forEach((rect, index) => {
    const before = during.rects[index];
    expect(before).toBeDefined();
    expect(rect.x).toBeCloseTo(before?.x ?? 0, 0);
    expect(rect.y).toBeCloseTo(before?.y ?? 0, 0);
    expect(rect.width).toBeCloseTo(before?.width ?? 0, 0);
    expect(rect.height).toBeCloseTo(before?.height ?? 0, 0);
  });
  const pageBox = await page.locator("#pdf-selection-harness .page").boundingBox();
  const rightColumn = await page.locator('#pdf-selection-harness [data-column="right"]').boundingBox();
  if (pageBox === null || rightColumn === null) throw new Error("column layout is missing");
  expect(Math.max(...after.rects.map((rect) => rect.x + rect.width))).toBeLessThan(rightColumn.x);
});

test("uses the pointer half-glyph position as a before or after selection endpoint", async ({ page }) => {
  await installSelectionHarness(page);
  const line = page.locator('#pdf-selection-harness [data-column="left"]').first();
  const box = await line.boundingBox();
  if (box === null) throw new Error("selection line is not laid out");
  const text = "Left column alpha beta";
  const characterWidth = box.width / text.length;
  const point = (index: number, fraction: number): { readonly x: number; readonly y: number } => ({
    x: box.x + characterWidth * (index + fraction),
    y: box.y + box.height / 2,
  });

  const cAfter = point(5, 0.75);
  const uBefore = point(8, 0.25);
  await page.mouse.move(cAfter.x, cAfter.y);
  await page.mouse.down();
  await page.mouse.move(uBefore.x, uBefore.y, { steps: 6 });
  await page.mouse.up();
  const forward = await page.evaluate(() => (window as typeof window & {
    __pdfSelectionHarness: { getSelection(): { text: string } };
  }).__pdfSelectionHarness.getSelection().text);
  expect(forward).toBe("ol");

  await page.mouse.move(uBefore.x, uBefore.y);
  await page.mouse.down();
  await page.mouse.move(cAfter.x, cAfter.y, { steps: 6 });
  await page.mouse.up();
  const reverse = await page.evaluate(() => (window as typeof window & {
    __pdfSelectionHarness: { getSelection(): { text: string } };
  }).__pdfSelectionHarness.getSelection().text);
  expect(reverse).toBe("ol");
});

test("commits a drag from its release coordinates when WebKit coalesces pointer moves", async ({ page }) => {
  await installSelectionHarness(page);
  await page.evaluate(() => {
    const line = document.querySelector<HTMLElement>('#pdf-selection-harness [data-column="left"]');
    if (line === null) throw new Error("selection line is missing");
    const rect = line.getBoundingClientRect();
    const start = { x: rect.left + 3, y: rect.top + rect.height / 2 };
    const end = { x: rect.left + rect.width * 0.55, y: start.y };
    line.dispatchEvent(new PointerEvent("pointerdown", {
      bubbles: true,
      button: 0,
      buttons: 1,
      clientX: start.x,
      clientY: start.y,
      isPrimary: true,
      pointerId: 41,
    }));
    window.document.dispatchEvent(new PointerEvent("pointerup", {
      bubbles: true,
      button: 0,
      clientX: end.x,
      clientY: end.y,
      isPrimary: true,
      pointerId: 41,
    }));
  });
  const result = await page.evaluate(() => {
    const harness = (window as typeof window & {
      __pdfSelectionHarness: { getCompleted(): number; getSelection(): { text: string } };
    }).__pdfSelectionHarness;
    return { completed: harness.getCompleted(), text: harness.getSelection()?.text };
  });
  expect(result.completed).toBe(1);
  expect(result.text?.length).toBeGreaterThan(1);
});

test("keeps PDF glyph geometry aligned after resize and ignores text-layer remounts", async ({ page }) => {
  await installSelectionHarness(page);
  const line = page.locator('#pdf-selection-harness [data-column="left"]').first();
  const box = await line.boundingBox();
  if (box === null) throw new Error("selection line is not laid out");
  await page.mouse.move(box.x + 3, box.y + box.height / 2);
  await page.mouse.down();
  await page.mouse.move(box.x + box.width * 0.7, box.y + box.height / 2, { steps: 6 });
  await page.mouse.up();
  const before = await probe(page);

  const versions = await page.evaluate(() => {
    const harness = (window as typeof window & {
      __pdfSelectionHarness: {
        store: { syncPage(page: HTMLElement): { version: number } | undefined };
        page: HTMLElement;
        textLayer: HTMLElement;
      };
    }).__pdfSelectionHarness;
    const firstVersion = Number(harness.page.dataset.pdfCharacterModelVersion);
    harness.page.style.width = "900px";
    harness.page.style.height = "600px";
    for (const span of harness.textLayer.querySelectorAll<HTMLElement>("span")) {
      span.style.left = `${Number.parseFloat(span.style.left) * 1.25}px`;
      span.style.top = `${Number.parseFloat(span.style.top) * 1.25}px`;
      span.style.fontSize = "25px";
    }
    const resizedVersion = harness.store.syncPage(harness.page)?.version ?? 0;
    const replacement = harness.textLayer.cloneNode(true) as HTMLElement;
    harness.textLayer.replaceWith(replacement);
    harness.textLayer = replacement;
    const remountedVersion = harness.store.syncPage(harness.page)?.version ?? 0;
    return { firstVersion, resizedVersion, remountedVersion };
  });
  expect(versions.resizedVersion).toBeGreaterThan(versions.firstVersion);
  expect(versions.remountedVersion).toBe(versions.resizedVersion);

  const after = await probe(page);
  expect(after.rects).toHaveLength(before.rects.length);
  after.rects.forEach((rect, index) => {
    const previous = before.rects[index];
    expect(previous).toBeDefined();
    expect(rect.x).toBeCloseTo(80 + ((previous?.x ?? 80) - 80) * 1.25, 0);
    expect(rect.y).toBeCloseTo(40 + ((previous?.y ?? 40) - 40) * 1.25, 0);
    expect(rect.width).toBeCloseTo((previous?.width ?? 0) * 1.25, 0);
    expect(rect.height).toBeCloseTo((previous?.height ?? 0) * 1.25, 0);
  });
});

test("selects adjacent PDF pages in reading order in both drag directions", async ({ page }) => {
  await installSelectionHarness(page);
  await page.evaluate(() => {
    const harness = (window as typeof window & {
      __pdfSelectionHarness: {
        store: { setPageCharacters(pageIndex: number, chars: readonly unknown[]): void };
        page: HTMLElement;
      };
    }).__pdfSelectionHarness;
    const viewer = harness.page.parentElement;
    if (viewer === null) throw new Error("selection viewer is missing");
    const secondPage = document.createElement("div");
    secondPage.className = "page";
    secondPage.dataset.pageNumber = "2";
    Object.assign(secondPage.style, {
      position: "absolute",
      left: "80px",
      top: "540px",
      width: "720px",
      height: "160px",
      background: "white",
      border: "0", // Synthetic glyphs use the full page box; real PDF border geometry is tested separately.
    });
    const textLayer = document.createElement("div");
    textLayer.className = "textLayer";
    Object.assign(textLayer.style, { position: "absolute", inset: "0" });
    const span = document.createElement("span");
    span.textContent = "Second page evidence";
    Object.assign(span.style, { position: "absolute", left: "40px", top: "45px", font: "20px/1.05 Georgia, serif" });
    textLayer.append(span);
    const highlightLayer = document.createElement("div");
    highlightLayer.className = "pdf-view-overlay-layer";
    secondPage.append(textLayer, highlightLayer);
    viewer.append(secondPage);
    const pageRect = secondPage.getBoundingClientRect();
    const lineRect = span.getBoundingClientRect();
    const values = Array.from(span.textContent ?? "");
    const width = lineRect.width / values.length;
    harness.store.setPageCharacters(1, values.map((value, index) => ({
      value,
      rect: {
        x: (lineRect.left - pageRect.left + index * width) / pageRect.width,
        y: (lineRect.top - pageRect.top) / pageRect.height,
        width: width / pageRect.width,
        height: lineRect.height / pageRect.height,
      },
      rotation: 0,
    })));
  });
  const first = await page.locator('#pdf-selection-harness [data-column="left"]').first().boundingBox();
  const second = await page.locator('#pdf-selection-harness .page[data-page-number="2"] span').boundingBox();
  if (first === null || second === null) throw new Error("two-page selection fixture is not laid out");
  await page.mouse.move(first.x + first.width * 0.75, first.y + first.height / 2);
  await page.mouse.down();
  await page.mouse.move(second.x + second.width * 0.45, second.y + second.height / 2, { steps: 10 });
  await page.mouse.up();
  const forward = await page.evaluate(() => (window as typeof window & {
    __pdfSelectionHarness: { getSelection(): { segments: readonly { pageIndex: number; text: string }[] } };
  }).__pdfSelectionHarness.getSelection().segments.map((segment) => segment.pageIndex));
  expect(forward).toEqual([0, 1]);

  await page.mouse.move(second.x + second.width * 0.45, second.y + second.height / 2);
  await page.mouse.down();
  await page.mouse.move(first.x + first.width * 0.75, first.y + first.height / 2, { steps: 10 });
  await page.mouse.up();
  const reverse = await page.evaluate(() => (window as typeof window & {
    __pdfSelectionHarness: { getSelection(): { segments: readonly { pageIndex: number; text: string }[] } };
  }).__pdfSelectionHarness.getSelection().segments.map((segment) => segment.pageIndex));
  expect(reverse).toEqual([0, 1]);
});
