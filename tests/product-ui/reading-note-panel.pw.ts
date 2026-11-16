import { expect, test } from "@playwright/test";

test("reading notes preserve the PDF floating session and hand it to the workspace durably", async ({ page }) => {
  const failures: string[] = [];
  page.on("pageerror", error => failures.push(error.message));
  await page.route("**/*", route => {
    const url = new URL(route.request().url());
    if (url.origin !== new URL(test.info().project.use.baseURL as string).origin || /^\/(api|v1|rpc|tauri)(\/|$)/.test(url.pathname)) {
      failures.push(`Forbidden request: ${url.href}`); return route.abort();
    }
    return route.continue();
  });
  await page.goto("/?preview=app");
  const inspectResource = () => page.evaluate(async () => {
    const {inspectReadingNoteResource} = await import("/integration/web/src/app/notes/NoteManagerContinuity.browser.tsx");
    return inspectReadingNoteResource("Synthetic Research Preview");
  });
  await page.getByRole("row", { name: /Synthetic Research Preview/ }).dblclick();
  await expect(page.getByText("This PDF is a local sample for frontend interaction testing.", { exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "高亮与笔记", exact: true }).click();
  await page.locator(".annotation-list button").filter({ hasText: "这是一条合成批注" }).dblclick();
  const panel = page.getByRole("dialog", { name: "阅读笔记编辑面板", exact: true });
  await expect(panel).toBeVisible(); await expect(panel).toHaveAttribute("aria-modal", "false");
  const editor = panel.locator(".bn-editor"); await expect(editor).toBeVisible();
  expect((await panel.boundingBox())!.width).toBe(520);
  await expect(panel.getByRole("textbox", { name: "笔记标题", exact: true })).toHaveCount(0);
  await expect(panel.locator(".note-page-breadcrumbs, .note-properties-panel, .note-relations")).toHaveCount(0);
  await expect(panel.getByText("无选区页级笔记", { exact: true })).not.toBeVisible();
  const opened = await inspectResource();
  expect(opened.document.pageId).toBe(opened.pageId);
  const original = await editor.elementHandle();
  await editor.click(); await editor.pressSequentially("Reading document preserved across navigation");
  const saved = panel.getByRole("status", { name: "已保存", exact: true });
  await expect(saved).toBeVisible({ timeout: 15000 }); await expect(saved).toHaveText("");
  // Use the production clipboard importer and Core asset store, rather than
  // seeding a second document or substituting a manager/query implementation.
  await editor.press("ControlOrMeta+End"); await editor.press("Enter");
  await editor.evaluate(element => {
    const bytes = Uint8Array.from(atob("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jI1sAAAAASUVORK5CYII="), value => value.charCodeAt(0));
    const clipboard = new DataTransfer();
    clipboard.items.add(new File([bytes], "reading-figure.png", {type: "image/png"}));
    element.dispatchEvent(new ClipboardEvent("paste", {bubbles: true, cancelable: true, clipboardData: clipboard}));
  });
  await expect(editor.locator(".managed-note-asset img")).toBeVisible();
  await expect.poll(async () => (await inspectResource()).assets.length).toBe(1);
  await saved.hover(); await expect(page.getByRole("tooltip")).toHaveText("已保存");
  await editor.hover();
  await panel.getByRole("button", { name: "阅读笔记操作", exact: true }).click();
  await page.keyboard.press("Escape");
  await expect(panel).toBeVisible(); await expect(page.getByRole("menu", { name: "阅读笔记操作", exact: true })).toHaveCount(0);
  await panel.getByRole("button", { name: "阅读笔记操作", exact: true }).click();
  await page.getByRole("menu", { name: "阅读笔记操作", exact: true }).getByRole("menuitem", { name: "版本历史", exact: true }).click();
  const historyDialog = page.getByRole("dialog", { name: "版本历史", exact: true });
  await expect(historyDialog).toBeVisible();
  await historyDialog.getByRole("button", { name: "关闭版本历史", exact: true }).click();
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  await expect(editor).toContainText("Reading document preserved across navigation");
  expect(await original?.evaluate(element => element.isConnected)).toBe(true);
  const before = await panel.boundingBox();
  await panel.locator(":scope > header strong").hover(); await page.mouse.down(); await page.mouse.move(before!.x + 260, before!.y + 60); await page.mouse.up();
  expect((await panel.boundingBox())!.y).toBeGreaterThanOrEqual(35);
  const grip = panel.getByRole("separator", { name: "调整阅读笔记 se 大小", exact: true });
  const point = await grip.boundingBox(); await page.mouse.move(point!.x + 5, point!.y + 5); await page.mouse.down(); await page.mouse.move(point!.x + 70, point!.y + 30); await page.mouse.up();
  await page.screenshot({ path: ".tmp/reading-note-floating-panel.png" });
  await editor.click(); await editor.press("ControlOrMeta+End"); await editor.press("Enter"); await editor.pressSequentially("/");
  await expect(editor).toHaveAttribute("aria-expanded", "true");
  await editor.press("Escape"); await expect(panel).toBeVisible(); await expect(editor).toHaveAttribute("aria-expanded", "false");
  await expect(saved).toBeVisible();
  const beforeTransfer = await inspectResource();
  expect(beforeTransfer.pageId).toBe(opened.pageId);
  expect(beforeTransfer.noteId).toBe(opened.noteId);
  expect(beforeTransfer.history.length).toBeGreaterThan(0);
  expect(beforeTransfer.assets[0]!.bytes.length).toBeGreaterThan(0);
  const managerEntry = page.getByRole("complementary", {name: "笔记导航", exact: true})
    .getByRole("link", {name: `打开笔记：${beforeTransfer.catalogTitle}`, exact: true});
  await managerEntry.click({ position: { x: 20, y: 15 } });
  await expect(panel).toHaveCount(0);
  expect(await original?.evaluate(element => element.isConnected)).toBe(false);
  const inline = page.locator(".notes-content .bn-editor");
  await expect(inline).toContainText("Reading document preserved across navigation");
  await expect(page.locator(".notes-content .note-save-state")).toHaveAttribute("aria-label", "已保存");
  await expect(inline.locator(".managed-note-asset")).toHaveAttribute("data-asset-id", beforeTransfer.assets[0]!.id);
  expect(await inspectResource()).toEqual(beforeTransfer);
  await page.reload(); await page.getByRole("button", { name: "笔记", exact: true }).click();
  await managerEntry.click();
  await expect(inline).toContainText("Reading document preserved across navigation");
  await expect(panel).toHaveCount(0);
  await expect(inline.locator(".managed-note-asset img")).toBeVisible();
  expect(await inspectResource()).toEqual(beforeTransfer);
  const restoredEditor = await inline.elementHandle();
  const source = page.locator(".notes-content .reading-note-source-section");
  await source.locator("summary").click();
  await expect(source.locator("blockquote")).toBeVisible();
  expect(await restoredEditor?.evaluate(element => element.isConnected)).toBe(true);
  await source.locator("summary").click();
  await expect(source.locator("blockquote")).not.toBeVisible();
  await page.screenshot({ path: ".tmp/reading-note-right-pane.png" });
  await page.setViewportSize({ width: 375, height: 667 });
  await expect(inline).toBeVisible();
  await expect(page.locator(".notes-content .note-save-state")).toHaveAttribute("aria-label", "已保存");
  expect(failures).toEqual([]);
});
