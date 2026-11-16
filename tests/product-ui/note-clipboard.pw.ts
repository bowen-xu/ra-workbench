import { expect, test } from "@playwright/test";

if (process.env.NOTE_SELECTION_WEBKIT === "1") test.use({ browserName: "webkit" });

// The same production tree must behave identically in the app and mock harness.
const cases = ["/", "/?preview=app"].flatMap((url) => [false, true].map((htmlOnly) => ({ url, htmlOnly })));
for (const { url, htmlOnly } of cases) {
  test(`blank block selection with ${htmlOnly ? "standard HTML" : "all rich formats"} at ${url}`, async ({ page, browser }, testInfo) => {
    expect(browser.browserType().name()).toBe(process.env.NOTE_SELECTION_WEBKIT === "1" ? "webkit" : "chromium");
    await page.goto(url);
    await page.evaluate(() => localStorage.clear());
    await page.reload();
    await page.getByRole("button", { name: "收起主导航" }).click({ force: true });
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await page.getByRole("button", { name: "新建页面" }).first().click();
    const editor = page.locator(".note-editor:visible .bn-editor");
    await editor.click();
    await page.keyboard.type("First text");
    await page.keyboard.press("Enter");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Last text");
    const rows = editor.locator(".bn-block-content");
    await expect(rows).toHaveText(["First text", "", "Last text"]);
    await expect(rows.nth(1)).toHaveAttribute("data-note-empty-block", "true");
    const first = await rows.first().locator(".bn-inline-content").boundingBox();
    const last = await rows.last().locator(".bn-inline-content").boundingBox();
    if (!first || !last) throw new Error("Missing inline geometry");
    await page.mouse.move(first.x + 1, first.y + first.height / 2);
    await page.mouse.down();
    await page.mouse.move(last.x + last.width - 1, last.y + last.height / 2, { steps: 12 });
    await page.mouse.up();
    await expect.poll(() => page.evaluate(() => getSelection()?.toString())).toContain("Last text");
    expect(await rows.nth(1).locator(".bn-inline-content").evaluate((node) => getComputedStyle(node, "::selection").backgroundColor)).toBe("rgba(0, 0, 0, 0)");
    expect(await rows.first().locator(".bn-inline-content").evaluate((node) => getComputedStyle(node, "::selection").backgroundColor)).toBe("rgb(180, 213, 254)");
    await page.screenshot({ path: testInfo.outputPath("blank-text-selection.png") });

    // Dispatch the actual DOM clipboard pipeline without writing the user's OS clipboard.
    const copied = await editor.evaluate((node) => {
      const data = new DataTransfer();
      node.dispatchEvent(new ClipboardEvent("copy", { bubbles: true, cancelable: true, clipboardData: data }));
      return Object.fromEntries([...data.types].map((mime) => [mime, data.getData(mime)]));
    });
    expect(copied["text/plain"]).toBe("First text\n\nLast text");
    expect(copied["blocknote/html"]).toContain('data-content-type="paragraph"');
    expect(copied["blocknote/html"]).not.toContain("data-note-empty-block");

    // A structural marquee paints blank and nonblank members identically,
    // both while dragging the rectangle and after releasing it.
    await page.keyboard.press("ArrowRight");
    const editorBox = await editor.boundingBox();
    if (!editorBox) throw new Error("Missing editor geometry");
    await page.mouse.move(editorBox.x + editorBox.width - 4, first.y + 1);
    await page.mouse.down();
    await page.mouse.move(last.x + 40, last.y + last.height - 1, { steps: 12 });
    await expect(page.locator(".note-block-selection-rect")).toBeVisible();
    const outer = editor.locator(".bn-block-outer");
    for (const row of await outer.all()) await expect(row).toHaveCSS("background-color", "rgb(234, 242, 237)");
    await page.mouse.up();
    for (const row of await outer.all()) await expect(row).toHaveCSS("background-color", "rgb(234, 242, 237)");
    await page.screenshot({ path: testInfo.outputPath("blank-block-marquee-selection.png") });
    await page.keyboard.press("Escape");
    for (const row of await outer.all()) await expect(row).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");

    // Explicit Shift+handle selection must include the blank member semantically.
    await page.keyboard.press("ArrowRight");
    await page.keyboard.down("Shift");
    const ids = await editor.locator(".bn-block-outer[data-id]").evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-id")!));
    for (const id of ids) {
      await editor.locator(`.bn-block-outer[data-id="${id}"] > .bn-block > .bn-block-content`).hover();
      const handle = page.getByRole("button", { name: "拖动块；点击打开块菜单" });
      await expect(handle).toHaveAttribute("data-block-id", id);
      await handle.click();
    }
    await page.keyboard.up("Shift");
    await expect(page.locator(".note-block-selection-status")).toContainText("已选中 3 个块");
    for (const row of await outer.all()) await expect(row).toHaveCSS("background-color", "rgb(234, 242, 237)");
    expect(await outer.first().evaluate((node) => {
      const content = node.querySelector(".bn-block-content")!;
      const rect = node.getBoundingClientRect(); const inner = content.getBoundingClientRect();
      return { left: inner.left - rect.left, right: rect.right - inner.right };
    })).toEqual({ left: 8, right: 8 });
    const title = await page.locator(".note-document-header > input").boundingBox();
    expect(title?.x).toBe(first.x);
    await page.screenshot({ path: testInfo.outputPath("block-selection-insets.png") });

    const group = await editor.evaluate((node) => {
      const data = new DataTransfer();
      node.dispatchEvent(new ClipboardEvent("copy", { bubbles: true, cancelable: true, clipboardData: data }));
      return Object.fromEntries([...data.types].map((mime) => [mime, data.getData(mime)]));
    });
    expect(group["text/plain"]).toBe(copied["text/plain"]);
    if (htmlOnly) delete group["blocknote/html"];
    await page.keyboard.press("Escape");
    await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
    const previousSession = await page.locator(".note-editor[data-editor-instance]:visible").getAttribute("data-editor-instance");
    await page.getByRole("button", { name: "新建独立笔记", exact: true }).click();
    await expect(page.locator(".note-editor[data-editor-instance]:visible")).not.toHaveAttribute("data-editor-instance", previousSession!);
    await expect(rows).toHaveText([""]);
    await editor.click();
    await editor.evaluate((node, values) => {
      const data = new DataTransfer();
      for (const [mime, value] of Object.entries(values)) data.setData(mime, value);
      node.dispatchEvent(new ClipboardEvent("paste", { bubbles: true, cancelable: true, clipboardData: data }));
    }, group);
    await expect(rows).toHaveText(["First text", "", "Last text"]);
    await page.keyboard.press("ControlOrMeta+z");
    await expect(rows).toHaveText([""]);
  });
}
