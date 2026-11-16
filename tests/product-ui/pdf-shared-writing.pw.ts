import {referenceProjectPath} from "../ui/projectPaths";
import { navigationPdf } from "../ui/sharedPdfFixture";
import { expect, test } from "@playwright/test";

const fixture = referenceProjectPath("zotero-reader/pdfjs/pdf.js/test/pdfs/tracemonkey.pdf");
test("writing shares continuous glyph selection, copy, focused search and versioned viewport recovery", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  const errors: string[] = []; page.on("pageerror", (error) => errors.push(error.message));
  await page.route("**/__shared.pdf", (route) => route.fulfill({ path: fixture, contentType: "application/pdf" }));
  await page.route("**/__short.pdf", (route) => route.fulfill({ body: navigationPdf(), contentType: "application/pdf" }));
  // Match WKWebView versions that have ReadableStream.getReader but no async iterator.
  await page.addInitScript(() => { Object.defineProperty(ReadableStream.prototype, Symbol.asyncIterator, { value: undefined, configurable: true }); });
  await page.goto("/");
  await page.evaluate(async () => {
    // The production preview lives in a single column inside the application
    // shell and relies on the workspace's shared styles, including PDF sizing.
    await import("/plugins/ra-writing-projects/src/features/writing/WritingWorkspace.css");
    await import("/plugins/ra-writing-projects/src/features/writing/WritingWorkbench.css");
    await import("/plugins/ra-writing-projects/src/features/writing/WritingLayout.css");
    await import("/plugins/ra-writing-projects/src/features/writing/preview/PreviewToolbar.css");
    const { default: React } = await import("/node_modules/.vite/deps/react.js");
    const { default: { createRoot } } = await import("/node_modules/.vite/deps/react-dom_client.js");
    const { CompilePanel } = await import("/plugins/ra-writing-projects/src/features/writing/preview/CompilePanel.tsx");
    const bytes = new Uint8Array(await fetch("/__shared.pdf").then((r) => r.arrayBuffer()));
    const host = document.createElement("div"); host.className = "app-shell";
    Object.assign(host.style, { position: "fixed", inset: "0", gridTemplateColumns: "minmax(0, 1fr)", gridTemplateRows: "minmax(0, 1fr)" });
    const column = document.createElement("aside"); column.className = "writing-preview-column";
    host.append(column);
    document.body.replaceChildren(host);
    const root = createRoot(column);
    const probe = { point: undefined as unknown, revision: undefined as string | undefined, inverse: [] as unknown[], bytes, refresh: (_revision: string, _bad = false) => {} };
    const onReadingPosition = (value: unknown) => { probe.point = value; };
    const onDisplayedVersion = (value: string | undefined) => { probe.revision = value; };
    probe.refresh = (revision, bad = false) => root.render(React.createElement(CompilePanel, {
      pdfIdentity: { key: "writing-fixture", revision }, pdfBytes: bad ? new Uint8Array([1, 2, 3]) : probe.bytes, active: true,
      isCompiling: false, toolchain: { available: true, tools: [] }, autoCompile: false, onAutoCompileChange: () => {},
      compilePreferences: { draft: false, codeCheck: true, stopOnFirstError: false }, onCompilePreferencesChange: () => {},
      onCompileFromScratch: () => {}, onCompile: () => {}, onCancel: () => {}, onOpenDiagnostic: () => {},
      syncActions: { backwardDisabled: "Fixture", forwardDisabled: "Fixture" },
      onReadingPosition, onDisplayedVersion, onInverseSearch: (...args: unknown[]) => probe.inverse.push(args),
    }));
    (window as unknown as { pdfProbe: typeof probe }).pdfProbe = probe;
    probe.refresh("one");
  });
  await expect(page.locator(".pdfViewer .page")).toHaveCount(14);
  // Portaled controls retain their visual styles without a toolbar ancestor.
  const zoomButton = page.getByRole("button", {name: "PDF 缩放", exact: true});
  await zoomButton.click();
  const zoom = page.getByRole("menuitem", {name: "150%", exact: true});
  await expect(zoom).toHaveCSS("height", "30px");
  await expect(page.getByRole("menu", {name: "PDF 缩放", exact: true})).toHaveCSS("background-color", "rgb(255, 255, 255)");
  await zoom.click();
  await expect(zoomButton).toHaveText("150%");
  await zoomButton.click();
  await page.getByRole("menuitem", {name: "适应宽度", exact: true}).click();
  await expect.poll(() => page.evaluate(() => (window as any).pdfProbe.revision)).toBe("one");
  const canvas = page.locator('.page[data-page-number="1"] canvas').first(); await expect(canvas).toBeVisible();
  await expect.poll(() => canvas.evaluate((c: HTMLCanvasElement) => c.width)).toBeGreaterThan(100);
  const geometry = await page.evaluate(async () => {
    const title = [...document.querySelectorAll<HTMLElement>('.page[data-page-number="1"] .textLayer span')].find((el) => el.textContent?.includes("Trace-based"))!;
    const r = title.getBoundingClientRect(); return { x: r.left + 2, y: r.top + r.height / 2, end: r.left + r.width * .7 };
  });
  expect(geometry.y).toBeGreaterThan(0);
  expect(geometry.y).toBeLessThan(page.viewportSize()!.height);
  await page.mouse.move(geometry.x, geometry.y); await page.mouse.down(); await page.mouse.move(geometry.end, geometry.y); await page.mouse.up();
  await expect(page.locator(".pending-selection-highlight").first()).toBeVisible();
  await test.info().attach("production-pdf-selection", { body: await page.screenshot(), contentType: "image/png" });
  await page.keyboard.press(process.platform === "darwin" ? "Meta+c" : "Control+c");
  expect(await page.evaluate(() => navigator.clipboard.readText())).toContain("Trace");
  expect(await page.evaluate(() => getSelection()?.toString())).toBe("");
  await page.mouse.dblclick(geometry.x + 30, geometry.y);
  await expect.poll(() => page.evaluate(() => (window as any).pdfProbe.inverse.length)).toBe(1);
  await page.locator(".pdf-view").focus(); await page.keyboard.press("Control+f");
  await expect(page.getByRole("search", { name: "搜索 PDF 内容" })).toBeVisible();
  await page.getByRole("textbox", { name: "PDF 搜索文字" }).fill("JavaScript");
  await expect.poll(() => page.locator(".pdf-view-search-options [aria-live]").textContent()).not.toBe("0 / 0");
  await page.keyboard.press("Escape"); await expect(page.getByRole("search")).toHaveCount(0);
  const pageInput = page.getByRole("textbox", { name: "PDF 页码" }); await pageInput.fill("4"); await pageInput.press("Enter");
  await expect(pageInput).toHaveValue("4");
  await page.getByRole("button", { name: "问题与日志（0）" }).click();
  await page.getByRole("menuitem", { name: "编译日志", exact: true }).click();
  await expect(page.locator(".writing-pdf-host")).toBeHidden();
  await page.getByRole("button", { name: "问题与日志（0）" }).click();
  await page.getByRole("menuitem", { name: "PDF 预览", exact: true }).click();
  await expect(pageInput).toHaveValue("4");
  // An open search must not rerun and navigate away after a compile replacement.
  await page.getByRole("button", { name: "搜索 PDF", exact: true }).click();
  await page.getByRole("textbox", { name: "PDF 搜索文字" }).fill("JavaScript");
  await pageInput.fill("4"); await pageInput.press("Enter");
  await page.locator(".pdf-view").evaluate((node) => { node.scrollTop += 180; });
  await page.waitForTimeout(250);
  const before = await page.evaluate(() => (window as any).pdfProbe.point);
  await page.evaluate(() => (window as any).pdfProbe.refresh("two"));
  await expect.poll(() => page.evaluate(() => (window as any).pdfProbe.revision)).toBe("two");
  await expect(pageInput).toHaveValue("4");
  await expect(page.getByRole("search")).toHaveCount(0);
  await expect.poll(() => page.evaluate(() => (window as any).pdfProbe.point?.y)).toBeCloseTo(before.y, 0);
  expect(await page.evaluate(() => (window as any).pdfProbe.bytes.byteLength)).toBeGreaterThan(1000);
  await page.evaluate(() => (window as any).pdfProbe.refresh("broken", true));
  await expect.poll(() => page.evaluate(() => (window as any).pdfProbe.revision)).toBeUndefined();
  await expect(page.locator('.page[data-page-number="4"] canvas').first()).toBeVisible();
  await page.evaluate(async () => {
    const probe = (window as any).pdfProbe;
    probe.bytes = new Uint8Array(await fetch("/__short.pdf").then((r) => r.arrayBuffer()));
    probe.refresh("short");
  });
  await expect.poll(() => page.evaluate(() => (window as any).pdfProbe.revision)).toBe("short");
  await expect(pageInput).toHaveValue("2");
  await page.setViewportSize({ width: 420, height: 740 });
  await page.getByRole("button", { name: "搜索 PDF", exact: true }).click();
  const popup = await page.getByRole("search").boundingBox();
  expect(popup!.x).toBeGreaterThanOrEqual(0); expect(popup!.x + popup!.width).toBeLessThanOrEqual(420);
  expect(errors).toEqual([]);
});
