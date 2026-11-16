import { expect, test, type Locator, type Page } from "@playwright/test";

const handleName = "拖动块；点击打开块菜单";
const highlightColor = "rgb(234, 242, 237)";

async function openNestedNote(page: Page): Promise<string[]> {
  await page.goto("/");
  await page.evaluate(() => window.localStorage.clear());
  await page.reload();
  await page.getByRole("button", { name: "收起主导航" }).click({ force: true });
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await page.getByRole("button", { name: "新建页面" }).first().click();
  await page.locator(".bn-editor").click();
  await page.keyboard.type("Parent");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Child");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.type('" Quote');
  await page.keyboard.press("Enter");
  await page.keyboard.type("Tail");
  await expect(page.locator(".bn-block-content:not([data-content-type=documentTitle])")).toHaveText(["Parent", "Child", "Quote", "Tail"]);
  // Waiting for the real debounce also separates fixture typing from drag undo.
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  const ids = await blockIds(page);
  expect(ids).toHaveLength(4);
  return ids;
}

function blockIds(page: Page): Promise<string[]> {
  return page.locator(".bn-editor .bn-block[data-id]").evaluateAll((nodes) => nodes.filter(node=>node.querySelector<HTMLElement>(":scope > .bn-block-content")?.dataset.contentType !== "documentTitle").map((node) => node.getAttribute("data-id")!));
}

async function startCapturedDrag(page: Page, sourceId: string): Promise<{
  handle: Locator;
  point: { x: number; y: number };
}> {
  await page.locator(`.bn-block[data-id="${sourceId}"] > .bn-block-content`).hover();
  const handle = page.getByRole("button", { name: handleName });
  await expect(handle).toHaveAttribute("data-block-id", sourceId);
  await handle.evaluate((element) => {
    element.addEventListener("gotpointercapture", (event) => {
      // Test-owned metadata is on the control, never ProseMirror content.
      element.setAttribute("data-test-pointer-id", String((event as PointerEvent).pointerId));
    }, { once: true });
  });
  const source = await handle.boundingBox();
  const target = await page.locator(".bn-block-content:not([data-content-type=documentTitle])").filter({ hasText: /^Tail$/ }).boundingBox();
  if (source === null || target === null) throw new Error("Missing drag geometry");
  const point = { x: source.x + 16, y: target.y + target.height - 2 };
  await page.mouse.move(source.x + 16, source.y + 16);
  await page.mouse.down();
  // Keep the pointer in the handle gutter, including the final release.
  await page.mouse.move(point.x, point.y, { steps: 12 });
  await expect(page.locator(".note-block-drop-cursor")).toBeVisible();
  await expect(handle).toHaveAttribute("data-test-pointer-id", /^\d+$/);
  expect(await handle.evaluate((element) => element.hasPointerCapture(Number(element.getAttribute("data-test-pointer-id"))))).toBe(true);
  return { handle, point };
}

async function injectButtonsZero(handle: Locator, point: { x: number; y: number }): Promise<void> {
  await handle.evaluate((element, coordinates) => {
    const pointerId = Number(element.getAttribute("data-test-pointer-id"));
    if (!element.hasPointerCapture(pointerId)) throw new Error("Expected a real captured pointer before fault injection");
    // Chromium fault injection of the observed native sequence: down(1),
    // move(1), move(0), then a real mouseup. This is not native acceptance.
    element.dispatchEvent(new PointerEvent("pointermove", {
      bubbles: true, cancelable: true, pointerId, pointerType: "mouse", isPrimary: true,
      clientX: coordinates.x, clientY: coordinates.y, button: -1, buttons: 0,
    }));
  }, point);
  expect(await handle.evaluate((element) => element.hasPointerCapture(Number(element.getAttribute("data-test-pointer-id"))))).toBe(true);
}

async function expectStillDragging(page: Page, originalIds: string[]): Promise<void> {
  await expect(page.locator(".note-editor[data-editor-instance]")).toHaveAttribute("data-block-dragging", "true");
  await expect(page.locator(".note-block-drop-cursor")).toBeVisible();
  await expect.poll(() => blockIds(page)).toEqual(originalIds);
}

test("a zero-buttons move preserves the captured gesture until one real gutter release", async ({ page }) => {
  const initial = await openNestedNote(page);
  const { handle, point } = await startCapturedDrag(page, initial[0]!);
  await page.screenshot({path:".tmp/verification/note-shared-block-drag.png"});
  await injectButtonsZero(handle, point);
  await expectStillDragging(page, initial);
  const subtree = page.locator(`.bn-block-outer[data-id="${initial[0]}"]`);
  await expect(subtree.locator(".bn-block-content")).toHaveText(["Parent", "Child"]);
  await expect(subtree).toHaveCSS("background-color", highlightColor);

  await page.mouse.up();
  const moved = [initial[2]!, initial[3]!, initial[0]!, initial[1]!];
  await expect.poll(() => blockIds(page)).toEqual(moved);
  await expect(page.locator(".note-block-drop-cursor")).toHaveCount(0);
  await expect(page.locator(".note-block-pointer-preview")).toHaveCount(0);
  await expect(subtree).toHaveCSS("background-color", highlightColor);
  await expect(subtree.locator(".bn-block-content")).toHaveText(["Parent", "Child"]);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => blockIds(page)).toEqual(initial);
  await expect(subtree).toHaveCSS("background-color", highlightColor);
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect.poll(() => blockIds(page)).toEqual(moved);
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  await expect(subtree).toHaveCSS("background-color", highlightColor);

  await page.reload();
  await page.getByRole("button", { name: "收起主导航" }).click({ force: true });
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await page.getByRole("tab", { name: "独立笔记", exact: true }).click();
  await page.getByRole("link", { name: "打开笔记：无标题", exact: true }).click();
  await expect.poll(() => blockIds(page)).toEqual(moved);
  await expect(subtree.locator(".bn-block-content")).toHaveText(["Parent", "Child"]);
});

for (const cancellation of ["Escape", "pointercancel", "outside"] as const) {
  test(`a zero-buttons move does not commit before ${cancellation} cancellation`, async ({ page }) => {
    const initial = await openNestedNote(page);
    const { handle, point } = await startCapturedDrag(page, initial[0]!);
    await injectButtonsZero(handle, point);
    await expectStillDragging(page, initial);

    if (cancellation === "Escape") await page.keyboard.press("Escape");
    else if (cancellation === "outside") await page.mouse.move(10, 10);
    else await handle.evaluate((element) => {
      element.dispatchEvent(new PointerEvent("pointercancel", {
        bubbles: true, pointerId: Number(element.getAttribute("data-test-pointer-id")),
        pointerType: "mouse", isPrimary: true,
      }));
    });
    await page.mouse.up();
    await expect.poll(() => blockIds(page)).toEqual(initial);
    await expect(page.locator(".note-editor[data-editor-instance]")).not.toHaveAttribute("data-block-dragging");
    await expect(page.locator(".note-block-drop-cursor")).toHaveCount(0);
    await expect(page.locator(".note-block-pointer-preview")).toHaveCount(0);
    await expect(page.locator("style[data-note-block-highlight]")).toHaveCount(0);
    await expect(page.getByRole("menu")).toHaveCount(0);
  });
}
