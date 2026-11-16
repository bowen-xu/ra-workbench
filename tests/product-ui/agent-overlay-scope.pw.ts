import { expect, test, type Locator } from "@playwright/test";
import { editorOverlay } from "./editorOverlay";

async function expectReachable(target: Locator): Promise<void> {
  await expect(target).toBeVisible();
  await expect.poll(() => target.evaluate(element => {
    const box = element.getBoundingClientRect();
    const hit = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
    return hit !== null && element.contains(hit);
  })).toBe(true);
}

test("the chat covers the intersecting portion of a visible note handle while hover stays on the note", async ({ page }) => {
  await page.goto("/?demo=text-patch");
  await page.getByRole("button", { name: "隐藏对话", exact: true }).click();
  const editor = page.getByLabel("笔记正文", { exact: true });
  const row = editor.locator('.bn-block[data-id="preview-argument"] > .bn-block-content');
  await row.hover();
  const overlay = await editorOverlay(page, editor);
  const handle = overlay.getByRole("button", { name: "拖动块；点击打开块菜单", exact: true });
  const handleBox = await handle.boundingBox(), rowBox = await row.boundingBox();
  if (!handleBox || !rowBox) throw new Error("Missing note control geometry");
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  const panel = page.locator(".writing-agent-panel:visible");
  const resizer = page.getByRole("separator", { name: "调整研究助手高度", exact: true });
  const resizeBox = await resizer.boundingBox();
  if (!resizeBox) throw new Error("Missing chat resize control");
  const top = handleBox.y + 12;
  await page.mouse.move(resizeBox.x + resizeBox.width / 2, resizeBox.y + resizeBox.height / 2);
  await page.mouse.down();
  await page.mouse.move(resizeBox.x + resizeBox.width / 2, top + resizeBox.height / 2, { steps: 12 });
  await page.mouse.up();
  // Production resizing snaps to size presets. Check the resulting overlap,
  // rather than assuming a freeform pixel height.
  await expect(panel).toHaveCSS("height", `${await resizer.getAttribute("aria-valuenow")}px`);
  await expect.poll(async () => (await panel.boundingBox())!.y).toBeLessThan(handleBox.y + handleBox.height - 2);
  const chatTop = (await panel.boundingBox())!.y;
  expect(chatTop).toBeGreaterThan(rowBox.y + 1);
  await page.mouse.move(rowBox.x + 20, chatTop - 2);
  expect(await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest(".bn-block")?.getAttribute("data-id"),
    { x: rowBox.x + 20, y: chatTop - 2 })).toBe("preview-argument");
  await expect(handle).toBeVisible();
  await expect(handle).toHaveAttribute("data-block-id", "preview-argument");
  const chat = await panel.boundingBox(), button = await handle.boundingBox();
  if (!chat || !button) throw new Error("Missing overlapping controls");
  const point = { x: Math.max(chat.x + 4, button.x + button.width / 2), y: Math.max(chat.y + 2, button.y + button.height / 2) };
  expect(point.x).toBeLessThan(Math.min(chat.x + chat.width, button.x + button.width));
  expect(point.y).toBeLessThan(Math.min(chat.y + chat.height, button.y + button.height));
  expect(await page.evaluate(({ x, y }) => Boolean(document.elementFromPoint(x, y)?.closest(".writing-agent-panel")), point)).toBe(true);
  await page.screenshot({ path: ".tmp/agent-overlay-note-edge.png" });
  // The background remains interactive while the chat is open.
  await editor.locator(".bn-editor h2").first().click();
  await expect(editor.locator(".bn-editor")).toBeFocused();
});

for (const route of ["text-patch", "agent-canvas"]) {
  test(`${route} chat menus and hints belong to its scope and keep state across hide/reopen`, async ({ page }) => {
    await page.goto(`/?demo=${route}`);
    const panel = page.locator(".writing-agent-panel:visible");
    await expect(panel).toBeVisible();
    const draft = panel.getByRole("textbox").last();
    await draft.fill("保留草稿");
    const draftNode = await draft.elementHandle();
    const add = page.getByRole("button", { name: "添加上下文和工具", exact: true });
    await add.click();
    const action = page.getByRole("menuitem", { name: "概念网络检索", exact: true });
    await expectReachable(action);
    expect(await action.evaluate(element => element.closest('[data-overlay-scope="panel"]')?.classList.contains("agent-canvas-layer"))).toBe(true);
    expect(await action.evaluate(element => Boolean(element.closest(".writing-agent-panel")))).toBe(false);
    await action.click();
    await expect(page.getByRole("button", { name: "关闭概念网络检索", exact: true })).toBeVisible();
    await add.click();
    await page.keyboard.press("Escape");
    await expect(add).toBeFocused();
    const hide = page.getByRole("button", { name: /^(隐藏对话|隐藏 AI 写作助手)$/u });
    await hide.hover();
    const hint = page.getByRole("tooltip").filter({ hasText: "隐藏" });
    await expect(hint).toBeVisible();
    expect(await hint.evaluate(element => Boolean(element.closest('[data-overlay-scope="panel"]')))).toBe(true);
    await hint.evaluate(element => (element as HTMLElement).style.pointerEvents = "auto");
    await expectReachable(hint);
    await hint.evaluate(element => (element as HTMLElement).style.pointerEvents = "none");
    await page.screenshot({ path: `.tmp/agent-overlay-${route}-hint.png` });
    await hide.click();
    await page.getByRole("button", { name: "打开对话", exact: true }).click();
    await expect(draft).toHaveValue("保留草稿");
    expect(await draftNode?.evaluate(node => node.isConnected)).toBe(true);
    if (route === "text-patch") {
      await page.getByRole("button", { name: "上下文管理", exact: true }).click();
      const modal = page.getByRole("dialog", { name: "上下文管理", exact: true });
      await expectReachable(modal);
      await page.keyboard.press("Escape");
      await expect(modal).not.toBeVisible();
      await expectReachable(add);
    }
    await page.setViewportSize({ width: 760, height: 650 });
    await add.click();
    await expectReachable(action);
    const bounds = await action.boundingBox();
    expect(bounds!.x).toBeGreaterThanOrEqual(0);
    expect(bounds!.x + bounds!.width).toBeLessThanOrEqual(760);
    await page.keyboard.press("Escape");
  });
}
