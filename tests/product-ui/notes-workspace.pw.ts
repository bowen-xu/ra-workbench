import { expect, test } from "@playwright/test";

const fixtureTitle = "工作区回归笔记";
const bodyContentSelector = '.bn-editor:visible .bn-block-content:not([data-content-type="documentTitle"])';
const bodyBlockSelector = '.bn-editor:visible .bn-block[data-id]:not(:has(> .bn-block-content[data-content-type="documentTitle"]))';
const bodyOuterSelector = '.bn-editor:visible .bn-block-outer:not(:has(> .bn-block > .bn-block-content[data-content-type="documentTitle"]))';

async function openFreshNote(page: import("@playwright/test").Page): Promise<void> {
  await page.goto("/");
  await page.evaluate(() => window.localStorage.clear());
  await page.reload();
  await page.getByRole("button", { name: "收起主导航" }).click({ force: true });
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await page.locator(".notes-explorer").getByRole("button", { name: "新建独立笔记", exact: true }).click();
  await expect(page.getByLabel("笔记标题", { exact: true }).filter({ visible: true })).toBeVisible();
  await page.getByLabel("笔记标题", { exact: true }).filter({ visible: true }).fill(fixtureTitle);
  await expect(page.locator('.bn-editor > .bn-block-group > .bn-block-outer:first-child > .bn-block > .bn-block-content'))
    .toHaveAttribute("data-content-type", "documentTitle");
}

test("notes uses the fixed writing column and a progressive Chinese block menu", async ({ page }) => {
  await openFreshNote(page);

  await expect(page.getByLabel("笔记标题", { exact: true }).filter({ visible: true })).toHaveText(fixtureTitle);
  const titleBox = await page.locator('.bn-editor:visible .bn-block-content[data-content-type="documentTitle"]').boundingBox();
  expect(titleBox?.width).toBeGreaterThanOrEqual(758);
  expect(titleBox?.width).toBeLessThanOrEqual(762);
  await expect(page.locator(".notes-explorer")).toHaveCSS("width", "272px");
  await expect(page.getByRole("button", { name: "关闭笔记导航" })).toBeHidden();

  const editorRoot = page.locator(".bn-editor");
  await editorRoot.click();
  await expect(editorRoot).toHaveCSS("outline-style", "none");

  const firstBlock = page.locator(bodyOuterSelector).first();
  await firstBlock.hover();
  const addBlock = page.getByRole("button", { name: "添加块" });
  await expect(addBlock).toBeVisible();
  const hitBox = await addBlock.boundingBox();
  // Notes narrows the shared 32px control to its documented 24px gutter.
  expect(hitBox?.width).toBe(24);
  expect(hitBox?.height).toBe(32);
  await addBlock.click();
  await expect(page.getByText("段落", { exact: true })).toBeVisible();
  await expect(page.getByText("公式块", { exact: true })).toBeVisible();
  await expect(page.getByText("可折叠列表", { exact: true })).toBeVisible();
  await expect(page.getByText("音频", { exact: true })).toHaveCount(0);
});

test("notes turns the navigation into a drawer without horizontal overflow", async ({ page }) => {
  await openFreshNote(page);
  await page.setViewportSize({ width: 900, height: 720 });

  await expect(page.getByRole("button", { name: "打开笔记导航" })).toBeVisible();
  await expect(page.locator(".notes-explorer")).not.toHaveClass(/open/);
  await page.getByRole("button", { name: "打开笔记导航" }).click();
  await expect(page.locator(".notes-explorer")).toHaveClass(/open/);
  await expect(page.getByRole("button", { name: "关闭笔记导航" }).last()).toBeVisible();
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});

test("block handles are opaque even when background WebKit defers animation frames", async ({ page }) => {
  await openFreshNote(page);
  const block = await page.locator(bodyOuterSelector).first().boundingBox();
  if (block === null) throw new Error("Block geometry is unavailable");
  // This page is discarded after the test. Only defer rAF after the editor has
  // mounted; React/DOM events still run as in a background native WebView.
  await page.evaluate(() => { window.requestAnimationFrame = () => 0; });
  await page.mouse.move(block.x + 100, block.y + 10);
  const handle = page.getByRole("button", { name: "拖动块；点击打开块菜单" });
  await expect(handle).toHaveCount(1);
  const opacity = await handle.evaluate((element) => {
    let result = 1;
    for (let node: Element | null = element; node !== null; node = node.parentElement) {
      result *= Number(getComputedStyle(node).opacity);
    }
    return result;
  });
  expect(opacity).toBe(1);
});

test("dragging the block handle moves the block and persists the new order", async ({ page }) => {
  await openFreshNote(page);

  const editor = page.locator(".bn-editor");
  await editor.click();
  await editor.type("第一块");
  await editor.press("Enter");
  await editor.type("第二块");
  await editor.press("Enter");
  await editor.type("第三块");

  const blocks = page.locator(bodyOuterSelector);
  const ids = await page.locator(bodyBlockSelector).evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-id")));
  await blocks.first().hover();
  const handle = page.getByRole("button", { name: "拖动块；点击打开块菜单" });
  const handleBox = await handle.boundingBox();
  const targetBox = await blocks.nth(2).boundingBox();
  if (handleBox === null || targetBox === null) throw new Error("Drag geometry is unavailable");
  await page.mouse.move(handleBox.x + handleBox.width / 2, handleBox.y + handleBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(handleBox.x + handleBox.width / 2 + 8, handleBox.y + handleBox.height / 2 + 8, { steps: 4 });
  await page.mouse.move(targetBox.x + 200, targetBox.y + targetBox.height - 2, { steps: 12 });
  await expect.poll(() => blocks.nth(1).evaluate(element => new DOMMatrixReadOnly(getComputedStyle(element).transform).m42)).toBeLessThan(-10);
  // Letting adjacent blocks make space must not commit the document during hover.
  expect(await blocks.allTextContents()).toEqual(["第一块", "第二块", "第三块"]);
  await expect(blocks.first()).toHaveCSS("opacity", "0.35");
  await page.mouse.up();

  await expect.poll(async () => blocks.allTextContents(), { timeout: 3_000 }).toEqual(["第二块", "第三块", "第一块"]);
  await expect(page.locator(".note-block-pointer-preview")).toHaveCount(0);
  const movedIds = await page.locator(bodyBlockSelector).evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-id")));
  expect(movedIds).toEqual([ids[1], ids[2], ids[0]]);
  await editor.press("ControlOrMeta+z");
  await expect.poll(() => blocks.allTextContents()).toEqual(["第一块", "第二块", "第三块"]);
  await editor.press("ControlOrMeta+Shift+z");
  await expect.poll(() => blocks.allTextContents()).toEqual(["第二块", "第三块", "第一块"]);
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  await page.reload();
  await page.getByRole("tab", { name: fixtureTitle, exact: true }).click();
  await expect.poll(async () => page.locator(bodyOuterSelector).allTextContents()).toEqual(["第二块", "第三块", "第一块"]);
});

test("Escape cancels an active block gesture without opening the handle menu", async ({ page }) => {
  await openFreshNote(page);
  const editor = page.locator(".bn-editor");
  await editor.click();
  await editor.type("保留原位");
  await editor.press("Enter");
  await editor.type("目标块");
  const blocks = page.locator(bodyOuterSelector);
  await blocks.first().hover();
  const handle = await page.getByRole("button", { name: "拖动块；点击打开块菜单" }).boundingBox();
  const target = await blocks.nth(1).boundingBox();
  if (handle === null || target === null) throw new Error("Drag geometry is unavailable");
  await page.mouse.move(handle.x + 16, handle.y + 16);
  await page.mouse.down();
  await page.mouse.move(target.x + 100, target.y + target.height - 1, { steps: 12 });
  await expect(page.locator(".note-block-pointer-preview")).toBeVisible();
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await expect(page.locator(".note-block-pointer-preview")).toHaveCount(0);
  await expect(page.locator(".note-editor[data-editor-instance]")).not.toHaveAttribute("data-block-dragging");
  await expect.poll(() => blocks.allTextContents()).toEqual(["保留原位", "目标块"]);
});

for (const [kind, menu] of [["quote", "引用"], ["codeBlock", "代码块"], ["formulaBlock", "公式块"], ["table", "表格"]]) {
  test(`upward drop above ${kind} uses block boundaries at either horizontal edge`, async ({ page }) => {
    await openFreshNote(page);
    const editor = page.locator(".bn-editor");
    await editor.click();
    await page.keyboard.type("Before");
    await page.keyboard.press("Enter");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Source");
    // Click the empty row's text origin, not its wide placeholder/center.
    await page.locator(bodyContentSelector).nth(1).click({ position: { x: 4, y: 10 } });
    await page.keyboard.type("/");
    await page.getByText(menu!, { exact: true }).click();
    const target = page.locator(`[data-content-type="${kind}"]`);
    await expect(target).toBeVisible();
    if (kind === "quote" || kind === "codeBlock") await page.keyboard.type("Target");
    const blockIds = () => page.locator(bodyBlockSelector).evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-id")));
    const initialIds = await blockIds();
    expect(initialIds).toHaveLength(3);
    await expect(page.locator(bodyContentSelector).nth(1)).toHaveAttribute("data-content-type", kind!);
    const source = page.locator(`.bn-block[data-id="${initialIds[2]}"]`);
    // Leave an actual text selection/toolbar open, as after a prior drag/undo.
    await source.locator(".bn-inline-content").dblclick({ position: { x: 15, y: 10 } });
    await expect(page.locator(".bn-formatting-toolbar")).toBeVisible();
    for (const inset of [250, 15]) {
      await source.hover();
      const h = await page.getByRole("button", { name: "拖动块；点击打开块菜单" }).boundingBox();
      const t = await target.boundingBox();
      if (h === null || t === null) throw new Error("Drag geometry is unavailable");
      await page.mouse.move(h.x + 16, h.y + 16);
      await page.mouse.down();
      await page.mouse.move(t.x + inset, t.y + 4, { steps: 4 });
      await expect(page.locator(".bn-formatting-toolbar")).toBeHidden();
      const cursor = page.locator(".note-block-drop-cursor");
      await expect(cursor).toBeVisible();
      await expect.poll(async () => {
        const line = await cursor.boundingBox();
        const currentTarget = await target.boundingBox();
        return line === null || currentTarget === null ? Infinity : Math.abs(line.y + line.height / 2 - currentTarget.y);
      }).toBeLessThanOrEqual(4);
      await page.mouse.up();
      await expect.poll(blockIds).toEqual([initialIds[0], initialIds[2], initialIds[1]]);
      await editor.press("ControlOrMeta+z");
      await expect.poll(blockIds).toEqual(initialIds);
    }
  });
}

test("a handle moves only its block despite a cross-block selection and repeated drags", async ({ page }) => {
  await openFreshNote(page);
  const editor = page.locator(".bn-editor");
  await editor.click();
  await page.keyboard.type("Before");
  await page.keyboard.press("Enter");
  await page.keyboard.type('" Quote');
  await page.keyboard.press("Enter");
  await page.keyboard.type("Source");
  const blocks = page.locator(bodyContentSelector);
  const ids = () => page.locator(bodyBlockSelector).evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-id")));
  const originalIds = await ids();
  const before = blocks.filter({ hasText: "Before" });
  const quote = page.locator('[data-content-type="quote"]');
  await before.click({ position: { x: 10, y: 10 } });
  await page.keyboard.press("Home");
  await page.keyboard.down("Shift");
  await quote.click({ position: { x: 30, y: 10 } });
  await page.keyboard.up("Shift");
  await expect.poll(() => page.evaluate(() => {
    const selection = window.getSelection();
    return selection !== null && !selection.isCollapsed
      && selection.anchorNode?.parentElement?.closest(".bn-block-content")
        !== selection.focusNode?.parentElement?.closest(".bn-block-content");
  })).toBe(true);
  // Separate fixture typing from the move's undo step (editor history window).
  await page.waitForTimeout(600);
  const drag = async (sourceText: string, targetText: string, after = false) => {
    await blocks.filter({ hasText: sourceText }).hover({ position: { x: 15, y: 10 } });
    const h = await page.getByRole("button", { name: "拖动块；点击打开块菜单" }).boundingBox();
    const t = await blocks.filter({ hasText: targetText }).boundingBox();
    if (h === null || t === null) throw new Error("Drag geometry is unavailable");
    await page.mouse.move(h.x + 16, h.y + 16);
    await page.mouse.down();
    await page.mouse.move(t.x + 250, after ? t.y + t.height - 4 : t.y + 4, { steps: 8 });
    await page.mouse.up();
  };
  await drag("Before", "Source", true);
  await expect.poll(ids).toEqual([originalIds[1], originalIds[2], originalIds[0]]);
  await editor.press("ControlOrMeta+z");
  await expect.poll(ids).toEqual(originalIds);
  // Do not click the text to clear the selection between these gestures.
  await drag("Quote", "Before");
  await expect.poll(ids).toEqual([originalIds[1], originalIds[0], originalIds[2]]);
  await drag("Source", "Quote");
  await expect.poll(ids).toEqual([originalIds[2], originalIds[1], originalIds[0]]);
  await expect(page.locator(".bn-block-group .bn-block-group")).toHaveCount(0);
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  await page.reload();
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await page.getByRole("tab", { name: fixtureTitle, exact: true }).click();
  await expect.poll(ids).toEqual([originalIds[2], originalIds[1], originalIds[0]]);
  await expect.poll(() => blocks.allTextContents()).toEqual(["Source", "Quote", "Before"]);
});

test("block boundaries preserve existing nesting and the moved subtree", async ({ page }) => {
  await openFreshNote(page);
  const editor = page.locator(".bn-editor");
  await editor.click();
  await page.keyboard.type("Parent");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Child A");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Child B");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.type("Source");
  // Separate fixture typing from the drag in ProseMirror's 500ms history group.
  await page.waitForTimeout(600);
  const source = page.locator(bodyContentSelector).filter({ hasText: "Source" });
  await source.dblclick({ position: { x: 15, y: 10 } });
  await source.hover({ position: { x: 15, y: 10 } });
  const h = await page.getByRole("button", { name: "拖动块；点击打开块菜单" }).boundingBox();
  const t = await page.locator(bodyContentSelector).filter({ hasText: "Child B" }).boundingBox();
  if (h === null || t === null) throw new Error("Drag geometry is unavailable");
  await page.mouse.move(h.x + 16, h.y + 16);
  await page.mouse.down();
  await page.mouse.move(t.x + 200, t.y + 4, { steps: 8 });
  await page.mouse.up();
  const children = page.locator(".bn-editor > .bn-block-group > .bn-block-outer .bn-block-group .bn-block-content");
  await expect.poll(() => children.allTextContents()).toEqual(["Child A", "Source", "Child B"]);
  await editor.press("ControlOrMeta+z");
  await expect.poll(() => children.allTextContents()).toEqual(["Child A", "Child B"]);
  const ids = await page.locator(bodyBlockSelector).evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-id")));
  const parent = page.locator(bodyContentSelector).filter({ hasText: "Parent" });
  await parent.hover({ position: { x: 15, y: 10 } });
  const parentHandle = await page.getByRole("button", { name: "拖动块；点击打开块菜单" }).boundingBox();
  const after = await source.boundingBox();
  if (parentHandle === null || after === null) throw new Error("Drag geometry is unavailable");
  await page.mouse.move(parentHandle.x + 16, parentHandle.y + 16);
  await page.mouse.down();
  await page.mouse.move(after.x + 200, after.y + after.height - 4, { steps: 8 });
  await page.mouse.up();
  await expect.poll(() => page.locator(bodyBlockSelector).evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-id"))))
    .toEqual([ids[3], ids[0], ids[1], ids[2]]);
  await expect.poll(() => children.allTextContents()).toEqual(["Child A", "Child B"]);
});

test("gutter gestures can detach an existing child and move on the first release", async ({ page }) => {
  await openFreshNote(page);
  await page.locator(".bn-editor").click();
  await page.keyboard.type("hi");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Enter");
  await page.keyboard.type("this is a note");
  await page.keyboard.press("Enter");
  await page.keyboard.type("23");
  await page.keyboard.press("Enter");
  await page.keyboard.type("quote");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.type("Tail");
  const blocks = page.locator(bodyContentSelector);
  const dragInGutter = async (source: string, target: string) => {
    await page.mouse.move(0, 0);
    await blocks.filter({ hasText: source }).hover({ position: { x: 15, y: 10 } });
    const h = await page.getByRole("button", { name: "拖动块；点击打开块菜单" }).boundingBox();
    const t = await blocks.filter({ hasText: target }).boundingBox();
    if (h === null || t === null) throw new Error("Drag geometry is unavailable");
    await page.mouse.move(h.x + 16, h.y + 16);
    await page.mouse.down();
    await page.mouse.move(h.x + 16, t.y + 4, { steps: 12 });
    await page.mouse.up();
  };
  await dragInGutter("23", "this is a note");
  await expect.poll(() => blocks.allTextContents()).toEqual(["hi", "", "23", "quote", "this is a note", "Tail"]);
  await dragInGutter("this is a note", "quote");
  await expect.poll(() => blocks.allTextContents()).toEqual(["hi", "", "23", "this is a note", "quote", "Tail"]);
  await dragInGutter("quote", "23");
  await expect.poll(() => blocks.allTextContents()).toEqual(["hi", "", "quote", "23", "this is a note", "Tail"]);
  await expect.poll(() => page.locator(".bn-block-group .bn-block-group .bn-block-content").allTextContents()).toEqual(["this is a note"]);
});

test("recorded Markdown quote remains independent during consecutive gutter drags", async ({ page }) => {
  await openFreshNote(page);
  const markdown = "hi\n\n\n\nthis is a note\n\n23\n\n> quote\n\n\n\nToggle list\n\nitem1\n\nitem2\n\n\n\n* item1\n* item2\n* item3\n\n\n\n456\n\n123";
  page.on("dialog", (dialog) => dialog.accept());
  await page.locator('input[accept=".md,.markdown,text/markdown,text/plain"]').setInputFiles({
    name: "recorded-quote.md", mimeType: "text/markdown", buffer: Buffer.from(markdown),
  });
  await expect(page.getByLabel("笔记标题", { exact: true }).filter({ visible: true })).toHaveText("recorded-quote");
  const blocks = page.locator(bodyContentSelector);
  await expect(page.locator('[data-content-type="quote"]')).toHaveText("quote");
  await expect(page.locator(".bn-block-group .bn-block-group")).toHaveCount(0);
  const original = await blocks.allTextContents();
  const drag = async (source: string, target: string) => {
    await blocks.filter({ hasText: new RegExp(`^${source}$`) }).hover({ position: { x: 15, y: 10 } });
    const h = await page.getByRole("button", { name: "拖动块；点击打开块菜单" }).boundingBox();
    const t = await blocks.filter({ hasText: new RegExp(`^${target}$`) }).boundingBox();
    if (h === null || t === null) throw new Error("Drag geometry is unavailable");
    await page.mouse.move(h.x + 16, h.y + 16);
    await page.mouse.down();
    await page.mouse.move(h.x + 16, t.y + 4, { steps: 12 });
    await page.mouse.up();
  };
  await drag("23", "this is a note");
  const moved = ["hi", "23", "this is a note", ...original.slice(3)];
  await expect.poll(() => blocks.allTextContents()).toEqual(moved);
  await drag("quote", "23");
  await expect.poll(() => blocks.allTextContents()).toEqual(["hi", "quote", "23", "this is a note", ...original.slice(4)]);
});

test("a completed handle move restores editing focus before keyboard undo", async ({ page }) => {
  await openFreshNote(page);
  const editor = page.locator(".bn-editor");
  await editor.click();
  await page.keyboard.type("Source");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Target");
  await page.waitForTimeout(600);
  const title = page.getByLabel("笔记标题", { exact: true }).filter({ visible: true });
  const initialTitle = await title.innerText();
  await title.click();
  const blocks = page.locator(bodyContentSelector);
  await blocks.filter({ hasText: /^Source$/ }).hover();
  const h = await page.getByRole("button", { name: "拖动块；点击打开块菜单" }).boundingBox();
  const t = await blocks.filter({ hasText: /^Target$/ }).boundingBox();
  if (h === null || t === null) throw new Error("Drag geometry is unavailable");
  await page.mouse.move(h.x + 16, h.y + 16);
  await page.mouse.down();
  await page.mouse.move(t.x + 200, t.y + t.height - 3, { steps: 8 });
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await expect(editor).toBeFocused();
  await expect.poll(() => title.evaluate(element => element.contains(document.getSelection()?.anchorNode ?? null))).toBe(true);
  await expect.poll(() => blocks.allTextContents()).toEqual(["Source", "Target"]);
  await page.mouse.move(h.x + 16, h.y + 16);
  await page.mouse.down();
  await page.mouse.move(t.x + 200, t.y + t.height - 3, { steps: 8 });
  await page.mouse.up();
  await expect.poll(() => blocks.allTextContents()).toEqual(["Target", "Source"]);
  await expect(editor).toBeFocused();
  // page.keyboard deliberately does not focus the editor for the application.
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => blocks.allTextContents()).toEqual(["Source", "Target"]);
  await expect(title).toHaveText(initialTitle);
});

test("dragging highlights the complete subtree without quote-like nesting rules or selection coupling", async ({ page }) => {
  await openFreshNote(page);
  const editor = page.locator(".bn-editor");
  await editor.click();
  await page.keyboard.type("Parent");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Child");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.type('" Independent quote');
  await page.keyboard.press("Enter");
  await page.keyboard.type("Tail");
  const blocks = page.locator(bodyContentSelector);
  const ids = () => page.locator(bodyBlockSelector).evaluateAll((nodes) => nodes.map((node) => node.getAttribute("data-id")));
  const initial = await ids();
  expect(initial).toHaveLength(4);
  const nested = page.locator(".bn-block-group .bn-block-group > .bn-block-outer");
  await expect(nested).toHaveCount(1);
  expect(await nested.evaluate((element) => getComputedStyle(element, "::before").display)).toBe("none");
  const quote = page.locator('[data-content-type="quote"]');
  await expect(quote).toHaveText("Independent quote");
  // Let fixture typing finish its separate editor history group.
  await page.waitForTimeout(600);
  await blocks.filter({ hasText: /^Parent$/ }).hover();
  const handle = page.getByRole("button", { name: "拖动块；点击打开块菜单" });
  await expect(handle).toHaveAttribute("data-block-id", initial[0]!);
  const h = await handle.boundingBox();
  const t = await blocks.filter({ hasText: /^Tail$/ }).boundingBox();
  if (h === null || t === null) throw new Error("Drag geometry is unavailable");
  await page.mouse.move(h.x + 16, h.y + 16);
  await page.mouse.down();
  await page.mouse.move(h.x + 16, t.y + t.height - 2, { steps: 12 });
  const highlighted = page.locator(`.bn-block-outer[data-id="${initial[0]}"]`);
  await expect(highlighted).toHaveCount(1);
  await expect(highlighted.locator(".bn-block-content")).toHaveText(["Parent", "Child"]);
  await expect(highlighted).toHaveCSS("background-color", "rgb(234, 242, 237)");
  await expect(highlighted.locator('[data-content-type="quote"]')).toHaveCount(0);
  await page.screenshot({ path: ".tmp/note-subtree-drag.png" });
  // Native selection can change after drag activation. It must never redefine
  // the move source or the deletion range, as the old clipboard pipeline did.
  await editor.evaluate((element) => window.getSelection()?.selectAllChildren(element));
  await page.mouse.up();
  await expect.poll(ids).toEqual([initial[2], initial[3], initial[0], initial[1]]);
  await expect(highlighted).toHaveCSS("background-color", "rgb(234, 242, 237)");
  await expect(page.locator(".note-block-drop-cursor")).toHaveCount(0);
  await page.mouse.move(10, 10);
  await expect(handle).not.toBeVisible();
  await expect(highlighted).toHaveCSS("background-color", "rgb(234, 242, 237)");
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(ids).toEqual(initial);
  await expect(highlighted).toHaveCSS("background-color", "rgb(234, 242, 237)");
  await page.keyboard.press("ControlOrMeta+Shift+z");
  await expect.poll(ids).toEqual([initial[2], initial[3], initial[0], initial[1]]);
  await expect(highlighted).toHaveCSS("background-color", "rgb(234, 242, 237)");
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(ids).toEqual(initial);
  await blocks.filter({ hasText: /^Child$/ }).hover();
  let childHandle = await handle.boundingBox();
  let parent = await blocks.filter({ hasText: /^Parent$/ }).boundingBox();
  if (childHandle === null || parent === null) throw new Error("Drag geometry is unavailable");
  await page.mouse.move(childHandle.x + 16, childHandle.y + 16);
  await page.mouse.down();
  await page.mouse.move(childHandle.x + 16, parent.y + 2, { steps: 8 });
  await expect(page.locator(`.bn-block-outer[data-id="${initial[1]}"]`)).toHaveCSS("background-color", "rgb(234, 242, 237)");
  await expect(highlighted).not.toHaveCSS("background-color", "rgb(234, 242, 237)");
  await page.mouse.move(10, 10);
  await page.mouse.up();
  await expect.poll(ids).toEqual(initial);
  await expect(highlighted).toHaveCSS("background-color", "rgb(234, 242, 237)");
  await blocks.filter({ hasText: /^Child$/ }).hover();
  await expect(handle).toHaveAttribute("data-block-id", initial[1]!);
  childHandle = await handle.boundingBox(); parent = await blocks.filter({ hasText: /^Parent$/ }).boundingBox();
  if (childHandle === null || parent === null) throw new Error("Drag geometry is unavailable");
  await page.mouse.move(childHandle.x + 16, childHandle.y + 16);
  await page.mouse.down();
  await page.mouse.move(childHandle.x + 16, parent.y + 2, { steps: 8 });
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await expect(page.locator("style[data-note-block-highlight]")).toHaveCount(0);
  await expect.poll(ids).toEqual(initial);
  await blocks.filter({ hasText: /^Child$/ }).hover();
  await expect(handle).toHaveAttribute("data-block-id", initial[1]!);
  childHandle = await handle.boundingBox(); parent = await blocks.filter({ hasText: /^Parent$/ }).boundingBox();
  if (childHandle === null || parent === null) throw new Error("Drag geometry is unavailable");
  await page.mouse.move(childHandle.x + 16, childHandle.y + 16);
  await page.mouse.down();
  await page.mouse.move(childHandle.x + 16, parent.y + 2, { steps: 8 });
  await page.mouse.up();
  await expect.poll(ids).toEqual([initial[1], initial[0], initial[2], initial[3]]);
  const child = page.locator(`.bn-block-outer[data-id="${initial[1]}"]`);
  await expect(child).toHaveCSS("background-color", "rgb(234, 242, 237)");
  await expect(highlighted).not.toHaveCSS("background-color", "rgb(234, 242, 237)");
  await page.getByLabel("笔记标题", { exact: true }).filter({ visible: true }).click();
  await expect(page.locator("style[data-note-block-highlight]")).toHaveCount(0);
});
