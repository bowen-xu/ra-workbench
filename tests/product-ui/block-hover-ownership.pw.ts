import { expect, test, type Page } from "@playwright/test";
import { editorOverlay } from "./editorOverlay";

async function mount(page: Page) {
  await page.goto("/");
  await page.evaluate(async () => {
    const { mountSharedEditorFixture } = await import("/ra-workbench/apps/web/src/core/components/block-editor/BlockEditor.fixture.tsx");
    mountSharedEditorFixture();
  });
  const host = page.getByRole("region", { name: "写作思路" });
  const surface = host.locator(".block-editor");
  const overlay = await editorOverlay(page, surface);
  return { host, surface, overlay, first: host.locator('.bn-block[data-id="first"] > .bn-block-content') };
}

test("an unrelated floating surface cannot activate background block controls, including from the outer gutter", async ({ page }) => {
  const { surface, overlay, first } = await mount(page);
  await first.hover();
  await expect(overlay.getByRole("button", { name: "添加块", exact: true })).toBeVisible();
  const editor = await surface.locator(".bn-editor").boundingBox();
  const row = await first.boundingBox();
  if (!editor || !row) throw new Error("Missing editor bounds");
  // Keep this synthetic panel below page portals to test hover ownership
  // independently of stacking. Its gutter exercises upstream's expanded search.
  const left = Math.max(0, editor.x - 90), top = row.y - 8;
  await page.evaluate(({ left, top }) => {
    const panel = document.createElement("aside");
    panel.id = "unrelated-surface";
    panel.textContent = "Independent floating surface";
    Object.assign(panel.style, { position: "fixed", left: `${left}px`, top: `${top}px`, width: "360px", height: "220px", zIndex: "40", background: "white", border: "1px solid black" });
    document.body.append(panel);
  }, { left, top });
  for (const x of [left + 8, editor.x + 100]) {
    await page.mouse.move(x, row.y + 10);
    await expect(overlay.locator(".bn-side-menu")).not.toBeVisible();
    const hit = await page.evaluate(({ x, y }) => document.elementFromPoint(x, y)?.closest("#unrelated-surface")?.id, { x, y: row.y + 10 });
    expect(hit).toBe("unrelated-surface");
  }
  await page.screenshot({ path: ".tmp/block-hover-ownership-occluded.png" });
  await page.locator("#unrelated-surface").evaluate(node => node.remove());
  await first.hover();
  const add = overlay.getByRole("button", { name: "添加块", exact: true });
  await expect(add).toBeVisible();
  await add.click();
  await expect(page.getByRole("listbox")).toBeVisible();
  await page.keyboard.press("Escape");
});

test("the real shared chat receives hover while the note behind it stays inactive", async ({ page }) => {
  await page.goto("/?demo=text-patch");
  await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill("改写研究问题");
  await page.getByRole("button", { name: "发送消息", exact: true }).click();
  await expect(page.locator(".writing-agent-turn-process").last().locator("summary").first()).toHaveAttribute("aria-label", /已完成/u);
  await page.getByRole("button", { name: "隐藏对话", exact: true }).click();
  const surface = page.getByLabel("笔记正文", { exact: true });
  const reviewed = surface.locator('.bn-block[data-id="preview-argument"] > .bn-block-content');
  await reviewed.hover();
  const overlay = await editorOverlay(page, surface);
  await expect(overlay.locator(".bn-side-menu")).toBeVisible();
  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  const panel = page.locator(".writing-agent-panel:visible");
  await expect(panel).toBeVisible();
  await panel.hover({ position: { x: 20, y: 70 } });
  await expect(overlay.locator(".bn-side-menu")).not.toBeVisible();
  await page.screenshot({ path: ".tmp/block-hover-ownership-chat.png" });
  await page.getByRole("button", { name: "隐藏对话", exact: true }).click();
  await reviewed.hover();
  await expect(overlay.locator(".bn-side-menu")).toBeVisible();
});

test("an active block drag survives crossing another editor and returning to its own drop target", async ({ page }) => {
  const { host, surface, overlay, first } = await mount(page);
  const parent = host.locator('.bn-block[data-id="parent"] > .bn-block-content');
  await parent.hover();
  const handle = overlay.getByRole("button", { name: "拖动块；点击打开块菜单", exact: true });
  const from = await handle.boundingBox(), to = await first.boundingBox();
  const other = await page.getByRole("region", { name: "临时草稿" }).locator(".bn-editor").boundingBox();
  if (!from || !to || !other) throw new Error("Missing drag geometry");
  await page.mouse.move(from.x + 16, from.y + 16);
  await page.mouse.down();
  await page.mouse.move(other.x + 100, other.y + 20, { steps: 12 });
  await expect(surface).toHaveAttribute("data-block-dragging", "true");
  await expect(overlay.locator(".bn-side-menu")).toBeVisible();
  await page.mouse.move(to.x + 4, to.y + 2, { steps: 12 });
  await page.mouse.up();
  await expect(host.locator(".bn-block-content").first()).toHaveText("Parent");
  await expect(surface).not.toHaveAttribute("data-block-dragging");
  await expect(page.locator(".note-block-pointer-preview")).toHaveCount(0);
  await expect(page.getByRole("region", { name: "临时草稿" }).locator(".bn-block-content").first()).toHaveText("First");
});
