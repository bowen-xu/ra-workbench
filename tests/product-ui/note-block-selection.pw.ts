import { expect, test, type Locator, type Page } from "@playwright/test";

const handleName = "拖动块；点击打开块菜单";
const highlightColor = "rgb(234, 242, 237)";
const bodyContentSelector = '.bn-editor .bn-block-content:not([data-content-type="documentTitle"])';
type BlockIdentity = { id: string; parentId: string | null };

async function blockTree(page: Page): Promise<BlockIdentity[]> {
  await expect(page.locator('.bn-editor > .bn-block-group > .bn-block-outer:first-child > .bn-block > .bn-block-content'))
    .toHaveAttribute("data-content-type", "documentTitle");
  return page.locator('.bn-editor .bn-block[data-id]:has(> .bn-block-content:not([data-content-type="documentTitle"]))').evaluateAll((nodes) => nodes.map((node) => ({
    id: node.getAttribute("data-id")!,
    parentId: node.parentElement?.closest(".bn-block[data-id]")?.getAttribute("data-id") ?? null,
  })));
}

function outer(page: Page, id: string): Locator {
  return page.locator(`.bn-block-outer[data-id="${id}"]`);
}

function content(page: Page, id: string): Locator {
  return page.locator(`.bn-block[data-id="${id}"] > .bn-block-content`);
}

async function openFixture(page: Page, targetKind: "quote" | "list" = "quote", options: {
  readonly url?: string;
  readonly expandedSidebar?: boolean;
} = {}): Promise<BlockIdentity[]> {
  await page.goto(options.url ?? "/");
  await page.evaluate(() => window.localStorage.clear());
  await page.reload();
  if (!options.expandedSidebar) await page.getByRole("button", { name: "收起主导航" }).click({ force: true });
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await page.getByRole("button", { name: "新建独立笔记", exact: true }).click();
  await page.locator(".bn-editor").click();
  await page.keyboard.type("First selection text");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Parent");
  await page.keyboard.press("Enter");
  await page.keyboard.type("Child");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Enter");
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.type("Peer");
  await page.keyboard.press("Enter");
  await page.keyboard.type(targetKind === "quote" ? '" Target' : "* Target");
  await page.keyboard.press("Enter");
  if (targetKind === "list") await page.keyboard.press("Backspace");
  await page.keyboard.type("Tail");
  await expect(page.locator(bodyContentSelector)).toHaveText(["First selection text", "Parent", "Child", "Peer", "Target", "Tail"]);
  await expect(page.locator(bodyContentSelector).nth(4)).toHaveAttribute("data-content-type", targetKind === "quote" ? "quote" : "bulletListItem");
  // Await actual persistence, which also separates fixture typing from move undo.
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  const tree = await blockTree(page);
  expect(tree).toHaveLength(6);
  expect(tree[2]!.parentId).toBe(tree[1]!.id);
  return tree;
}

async function beginMarquee(page: Page, tree: BlockIdentity[], side: "left" | "right"): Promise<void> {
  const parent = await content(page, tree[1]!.id).boundingBox();
  const peer = await content(page, tree[3]!.id).boundingBox();
  if (parent === null || peer === null) throw new Error("Missing marquee geometry");
  // Begin in the inter-block whitespace, not a rendered + / handle button.
  const startX = side === "left" ? parent.x - 40 : parent.x + parent.width + 20;
  const endX = side === "left" ? peer.x + 140 : peer.x + 4;
  await page.mouse.move(startX, parent.y - 1);
  await page.mouse.down();
  await page.mouse.move(endX, peer.y + peer.height - 2, { steps: 12 });
  await expect(page.locator(".note-block-selection-rect")).toBeVisible();
}

async function selectParentAndPeer(page: Page, tree: BlockIdentity[], side: "left" | "right"): Promise<void> {
  await beginMarquee(page, tree, side);
  await page.mouse.up();
  await expect(page.locator(".note-block-selection-rect")).toHaveCount(0);
  await expect(outer(page, tree[1]!.id)).toHaveCSS("background-color", highlightColor);
  await expect(outer(page, tree[3]!.id)).toHaveCSS("background-color", highlightColor);
  await expect(outer(page, tree[0]!.id)).not.toHaveCSS("background-color", highlightColor);
  await expect(outer(page, tree[4]!.id)).not.toHaveCSS("background-color", highlightColor);
  await expect.poll(() => blockTree(page)).toEqual(tree);
}

async function beginHandleDrag(page: Page, sourceId: string): Promise<{ x: number; y: number }> {
  await content(page, sourceId).hover();
  const handle = page.getByRole("button", { name: handleName });
  await expect(handle).toHaveAttribute("data-block-id", sourceId);
  const box = await handle.boundingBox();
  if (box === null) throw new Error("Missing handle geometry");
  const point = { x: box.x + 16, y: box.y + 16 };
  await page.mouse.move(point.x, point.y);
  await page.mouse.down();
  return point;
}

async function reopenSavedNote(page: Page): Promise<void> {
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  await page.reload();
  await page.getByRole("tab", { name: "无标题", exact: true }).click();
}

async function shiftSelectRoots(page: Page, ids: string[]): Promise<void> {
  // Press the modifier before approaching a handle: keyboard activity may
  // dismiss the previous hover menu, and the next hover should show it again.
  await page.keyboard.down("Shift");
  for (const id of ids) {
    await content(page, id).hover();
    const handle = page.getByRole("button", { name: handleName });
    await expect(handle).toHaveAttribute("data-block-id", id);
    await handle.click();
    await expect(outer(page, id)).toHaveCSS("background-color", highlightColor);
  }
  await page.keyboard.up("Shift");
}

for (const side of ["left", "right"] as const) {
  test(`${side} whitespace marquee moves the selected member group once with its subtree`, async ({ page }) => {
    const initial = await openFixture(page);
    await selectParentAndPeer(page, initial, side);
    // Starting from Peer must carry Parent too, in document order, with Child once.
    const handle = await beginHandleDrag(page, initial[3]!.id);
    const tail = await content(page, initial[5]!.id).boundingBox();
    if (tail === null) throw new Error("Missing target geometry");
    await page.mouse.move(handle.x, tail.y + tail.height - 2, { steps: 12 });
    await expect(page.locator(".note-block-pointer-preview")).toHaveText("移动 2 个块");
    await expect(page.locator(".note-block-drop-cursor")).toBeVisible();
    await page.mouse.up();
    const moved = [initial[0]!, initial[4]!, initial[5]!, initial[1]!, initial[2]!, initial[3]!];
    await expect.poll(() => blockTree(page)).toEqual(moved);
    await expect(outer(page, initial[1]!.id)).toHaveCSS("background-color", highlightColor);
    await expect(outer(page, initial[3]!.id)).toHaveCSS("background-color", highlightColor);
    await expect(outer(page, initial[1]!.id).locator(".bn-block-content")).toHaveText(["Parent", "Child"]);
    await page.keyboard.press("ControlOrMeta+z");
    await expect.poll(() => blockTree(page)).toEqual(initial);
    await page.keyboard.press("ControlOrMeta+Shift+z");
    await expect.poll(() => blockTree(page)).toEqual(moved);
    await reopenSavedNote(page);
    await expect.poll(() => blockTree(page)).toEqual(moved);
  });
}

for (const side of ["left", "right", "bottom"] as const) {
  test(`${side} outer whitespace starts a marquee in the preview with the navigation expanded`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 808 });
    const initial = await openFixture(page, "quote", { url: "/?preview=app", expandedSidebar: true });
    const parent = await content(page, initial[1]!.id).boundingBox();
    const peer = await content(page, initial[3]!.id).boundingBox();
    const pane = await page.locator(".notes-content").boundingBox();
    const editor = await page.locator(".bn-editor").boundingBox();
    if (!parent || !peer || !pane || !editor) throw new Error("Missing outer-whitespace geometry");
    const start = side === "bottom"
      ? { x: parent.x + parent.width / 2, y: editor.y + editor.height + 16 }
      : { x: side === "left" ? pane.x + 2 : pane.x + pane.width - 2, y: parent.y - 1 };
    expect(start.y).toBeLessThan(pane.y + pane.height);
    await page.mouse.move(start.x, start.y); await page.mouse.down();
    await page.mouse.move(parent.x + 140, side === "bottom" ? parent.y + 2 : peer.y + peer.height - 2, { steps: 12 });
    await expect(page.locator(".note-block-selection-rect")).toBeVisible();
    await expect(outer(page, initial[1]!.id)).toHaveCSS("background-color", highlightColor);
    await expect(outer(page, initial[3]!.id)).toHaveCSS("background-color", highlightColor);
    if (side === "bottom") await page.screenshot({ path: ".tmp/notes-bottom-marquee.png" });
    await page.mouse.up();
    await expect(page.locator(".note-block-selection-rect")).toHaveCount(0);
    await expect(outer(page, initial[1]!.id)).toHaveCSS("background-color", highlightColor);
    expect(await blockTree(page)).toEqual(initial);
    await page.keyboard.press("Escape");
    await expect(page.locator("style[data-note-block-highlight]")).toHaveCount(0);
  });
}

test("a content-origin drag remains text selection and does not make the handle carry that text range", async ({ page }) => {
  const initial = await openFixture(page);
  const first = await content(page, initial[0]!.id).boundingBox();
  const peer = await content(page, initial[3]!.id).boundingBox();
  if (first === null || peer === null) throw new Error("Missing text selection geometry");
  await page.mouse.move(first.x + 3, first.y + first.height / 2);
  await page.mouse.down();
  await page.mouse.move(peer.x + 20, peer.y + peer.height / 2, { steps: 12 });
  await expect(page.locator(".note-block-selection-rect")).toHaveCount(0);
  await page.mouse.up();
  expect(await page.evaluate(() => window.getSelection()?.toString())).toContain("Parent");
  await expect(page.locator("style[data-note-block-highlight]")).toHaveCount(0);
  const handle = await beginHandleDrag(page, initial[0]!.id);
  const tail = await content(page, initial[5]!.id).boundingBox();
  if (tail === null) throw new Error("Missing target geometry");
  await page.mouse.move(handle.x, tail.y + tail.height - 2, { steps: 12 });
  await expect(page.locator(".note-block-pointer-preview")).toHaveText("移动块");
  await page.mouse.up();
  await expect.poll(() => blockTree(page)).toEqual([...initial.slice(1), initial[0]!]);
});

for (const targetKind of ["quote", "list"] as const) {
  test(`central ${targetKind} target highlights its whole block and saves the nested move`, async ({ page }) => {
    const initial = await openFixture(page, targetKind);
    if (targetKind === "quote") await selectParentAndPeer(page, initial, "right");
    await beginHandleDrag(page, initial[targetKind === "quote" ? 3 : 1]!.id);
    const target = await content(page, initial[4]!.id).boundingBox();
    const targetOuter = await outer(page, initial[4]!.id).boundingBox();
    if (target === null || targetOuter === null) throw new Error("Missing inside target geometry");
    await page.mouse.move(target.x + 100, target.y + target.height / 2, { steps: 12 });
    const overlay = page.locator(`.note-block-drop-target[data-target-id="${initial[4]!.id}"]`);
    await expect(overlay).toBeVisible();
    await expect(page.locator(".note-block-pointer-preview")).toHaveText(`放入子块 · ${targetKind === "quote" ? 2 : 1} 个块`);
    const overlayBox = await overlay.boundingBox();
    expect(overlayBox).not.toBeNull();
    expect(overlayBox!.x).toBeCloseTo(targetOuter.x, 0);
    await expect.poll(async () => Math.abs((await outer(page, initial[4]!.id).boundingBox())!.y - overlayBox!.y)).toBeLessThan(1);
    expect(overlayBox!.width).toBeCloseTo(targetOuter.width, 0);
    expect(overlayBox!.height).toBeGreaterThan(targetOuter.height);
    await page.mouse.up();
    const nested = targetKind === "quote"
      ? [initial[0]!, initial[4]!, { ...initial[1]!, parentId: initial[4]!.id }, initial[2]!, { ...initial[3]!, parentId: initial[4]!.id }, initial[5]!]
      : [initial[0]!, initial[3]!, initial[4]!, { ...initial[1]!, parentId: initial[4]!.id }, initial[2]!, initial[5]!];
    await expect.poll(() => blockTree(page)).toEqual(nested);
    await expect(overlay).toHaveCount(0);
    await expect(outer(page, initial[1]!.id)).toHaveCSS("background-color", highlightColor);
    await page.keyboard.press("ControlOrMeta+z");
    await expect.poll(() => blockTree(page)).toEqual(initial);
    await page.keyboard.press("ControlOrMeta+Shift+z");
    await expect.poll(() => blockTree(page)).toEqual(nested);
    await reopenSavedNote(page);
    await expect.poll(() => blockTree(page)).toEqual(nested);
  });
}

test("Escape cancels an active marquee and then a selected-group drag without leftover feedback", async ({ page }) => {
  const initial = await openFixture(page);
  await beginMarquee(page, initial, "right");
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await expect(page.locator(".note-block-selection-rect")).toHaveCount(0);
  await expect(page.locator("style[data-note-block-highlight]")).toHaveCount(0);
  await expect.poll(() => blockTree(page)).toEqual(initial);

  await selectParentAndPeer(page, initial, "left");
  await beginHandleDrag(page, initial[3]!.id);
  const target = await content(page, initial[4]!.id).boundingBox();
  if (target === null) throw new Error("Missing inside target geometry");
  await page.mouse.move(target.x + 100, target.y + target.height / 2, { steps: 12 });
  await expect(page.locator(".note-block-drop-target")).toBeVisible();
  await page.keyboard.press("Escape");
  await page.mouse.up();
  await expect.poll(() => blockTree(page)).toEqual(initial);
  await expect(page.locator(".note-block-selection-rect")).toHaveCount(0);
  await expect(page.locator(".note-block-drop-target")).toHaveCount(0);
  await expect(page.locator(".note-block-drop-cursor")).toHaveCount(0);
  await expect(page.locator(".note-block-pointer-preview")).toHaveCount(0);
  await expect(page.locator("style[data-note-block-highlight]")).toHaveCount(0);
  await expect(page.getByRole("menu")).toHaveCount(0);
});

test("focused handle shortcuts and Shift-click toggle block selection without editing or opening a menu", async ({ page }) => {
  const initial = await openFixture(page);
  await content(page, initial[1]!.id).hover();
  const handle = page.getByRole("button", { name: handleName });
  await expect(handle).toHaveAttribute("data-block-id", initial[1]!.id);
  await handle.focus();
  await expect(handle).toBeFocused();

  await page.keyboard.press("Shift+Space");
  await expect(outer(page, initial[1]!.id)).toHaveCSS("background-color", highlightColor);
  await expect(handle).toBeFocused();
  await expect(page.getByRole("menu")).toHaveCount(0);
  await page.keyboard.press("Shift+Enter");
  await expect(outer(page, initial[1]!.id)).not.toHaveCSS("background-color", highlightColor);
  await expect(handle).toBeFocused();
  await page.keyboard.press("Shift+Space");
  await expect(outer(page, initial[1]!.id)).toHaveCSS("background-color", highlightColor);

  await content(page, initial[3]!.id).hover();
  await expect(handle).toHaveAttribute("data-block-id", initial[3]!.id);
  await handle.click({ modifiers: ["Shift"] });
  await expect(outer(page, initial[1]!.id)).toHaveCSS("background-color", highlightColor);
  await expect(outer(page, initial[3]!.id)).toHaveCSS("background-color", highlightColor);
  await expect(page.locator(".note-block-selection-rect")).toHaveCount(0);
  await expect(page.locator(".note-block-pointer-preview")).toHaveCount(0);
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect.poll(() => blockTree(page)).toEqual(initial);
  await expect(page.locator(bodyContentSelector)).toHaveText(["First selection text", "Parent", "Child", "Peer", "Target", "Tail"]);
  // The hidden DOM range now backs the engine's structural selection; its text
  // is not a separate caret/text selection or the source of the green paint.
  await expect(page.locator(".bn-editor")).toHaveClass(/ProseMirror-hideselection/);

  await page.keyboard.press("Escape");
  await expect(page.locator("style[data-note-block-highlight]")).toHaveCount(0);
  await expect(page.getByRole("menu")).toHaveCount(0);
  await expect.poll(() => blockTree(page)).toEqual(initial);
});

test("discrete selected roots can move into a sibling gap anchored by another selected root", async ({ page }) => {
  const initial = await openFixture(page);
  // A=First, B=Parent (including Child), C=Peer. B-after is canonically C-before,
  // but C's selection does not make grouping A+C after B a no-op or a cycle.
  await shiftSelectRoots(page, [initial[0]!.id, initial[3]!.id]);
  const handle = await beginHandleDrag(page, initial[0]!.id);
  const middle = await content(page, initial[1]!.id).boundingBox();
  if (middle === null) throw new Error("Missing sibling boundary geometry");
  await page.mouse.move(handle.x, middle.y + middle.height - 2, { steps: 12 });
  await expect(page.locator(".note-block-drop-cursor")).toBeVisible();
  await expect(page.locator(".note-block-pointer-preview")).toHaveText("移动 2 个块");
  await page.mouse.up();
  await expect.poll(() => blockTree(page)).toEqual([initial[1]!, initial[2]!, initial[0]!, initial[3]!, initial[4]!, initial[5]!]);
  await expect(outer(page, initial[0]!.id)).toHaveCSS("background-color", highlightColor);
  await expect(outer(page, initial[3]!.id)).toHaveCSS("background-color", highlightColor);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => blockTree(page)).toEqual(initial);
});

test("undoing an explicitly selected block creation leaves the surviving selected root draggable", async ({ page }) => {
  const initial = await openFixture(page);
  await content(page, initial[5]!.id).click({ position: { x: 120, y: 12 } });
  await page.keyboard.press("Enter");
  await page.keyboard.type("Ephemeral");
  await expect(page.locator(bodyContentSelector)).toHaveText(["First selection text", "Parent", "Child", "Peer", "Target", "Tail", "Ephemeral"]);
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  const withNewBlock = await blockTree(page);
  const ephemeral = withNewBlock[6]!;
  await shiftSelectRoots(page, [initial[0]!.id, ephemeral.id]);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => blockTree(page)).toEqual(initial);
  await expect(outer(page, initial[0]!.id)).toHaveCSS("background-color", highlightColor);

  const handle = await beginHandleDrag(page, initial[0]!.id);
  const tail = await content(page, initial[5]!.id).boundingBox();
  if (tail === null) throw new Error("Missing remaining root target geometry");
  await page.mouse.move(handle.x, tail.y + tail.height - 2, { steps: 12 });
  await expect(page.locator(".note-block-drop-cursor")).toBeVisible();
  await expect(page.locator(".note-block-pointer-preview")).toHaveText("移动块");
  await page.mouse.up();
  await expect.poll(() => blockTree(page)).toEqual([...initial.slice(1), initial[0]!]);
  await page.keyboard.press("ControlOrMeta+z");
  await expect.poll(() => blockTree(page)).toEqual(initial);
});
