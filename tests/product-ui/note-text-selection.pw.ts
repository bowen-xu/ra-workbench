import { expect, test } from "@playwright/test";

// Optional WebKit pixel inspection uses the same fixture without making a
// second browser installation mandatory for the regular Chromium suite.
if (process.env.NOTE_SELECTION_WEBKIT === "1") test.use({ browserName: "webkit" });

test("text selection paints inline content, not block gaps, without changing layout or editing", async ({ page }, testInfo) => {
  await page.goto("/");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await page.getByRole("button", { name: "收起主导航" }).click({ force: true });
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await page.getByRole("button", { name: "新建页面" }).first().click();
  const editor = page.locator(".bn-editor");
  await editor.click();
  await page.keyboard.type("Alpha");
  await page.keyboard.press("Enter");
  await page.keyboard.type('" Quote');
  await page.keyboard.press("Enter");
  await page.keyboard.type("* Item one");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Item two");
  await expect(page.locator(".bn-block-content")).toHaveText(["Alpha", "Quote", "Item one", "Item two"]);
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");

  const rows = page.locator(".bn-block-content");
  const geometry = () => rows.evaluateAll((nodes) => nodes.map((node) => {
    const rect = node.getBoundingClientRect();
    return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
  }));
  const before = await geometry();
  const first = await rows.first().locator(".bn-inline-content").boundingBox();
  const last = await rows.last().locator(".bn-inline-content").boundingBox();
  if (first === null || last === null) throw new Error("Missing inline selection geometry");
  await page.mouse.move(first.x + 1, first.y + first.height / 2);
  await page.mouse.down();
  await page.mouse.move(last.x + last.width - 1, last.y + last.height / 2, { steps: 15 });
  await page.mouse.up();
  await expect.poll(() => page.evaluate(() => getSelection()?.toString())).toContain("Item two");
  await expect(page.locator("style[data-note-block-highlight]")).toHaveCount(0);
  await expect(page.locator(".note-block-selection-rect")).toHaveCount(0);
  expect(await geometry()).toEqual(before);
  expect(await page.locator(".bn-block-outer").nth(1).evaluate((node) => getComputedStyle(node).marginTop)).toBe("4px");
  const wrappers = page.locator(".note-editor :is(.bn-editor, .bn-block-group, .bn-block-outer, .bn-block, .bn-block-content)");
  expect(await wrappers.evaluateAll((nodes) => nodes.every((node) => getComputedStyle(node, "::selection").backgroundColor === "rgba(0, 0, 0, 0)"))).toBe(true);
  expect(await rows.first().locator(".bn-inline-content").evaluate((node) => getComputedStyle(node, "::selection").backgroundColor)).toBe("rgb(180, 213, 254)");
  await page.screenshot({ path: testInfo.outputPath("text-selection.png") });

  const text = await editor.innerText();
  await page.keyboard.press("Backspace");
  await expect(editor).not.toHaveText(text);
  await page.keyboard.press("ControlOrMeta+z");
  await expect(rows).toHaveText(["Alpha", "Quote", "Item one", "Item two"]);
});
