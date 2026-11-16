import { expect, test, type Page } from "@playwright/test";
import { editorOverlay } from "./editorOverlay";

async function send(page: Page, text: string) {
  await page.getByRole("textbox", { name: "给研究助手发送消息", exact: true }).fill(text);
  await page.getByRole("button", { name: "发送消息", exact: true }).click();
  await expect(page.locator(".writing-agent-turn-process").last().locator("summary").first()).toHaveAttribute("aria-label", /已完成/u);
  await page.getByRole("button", { name: "隐藏对话", exact: true }).click();
}

test("review side controls start at the displayed old/new block while ordinary and added roots retain their own anchor", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/?demo=text-patch");
  await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("研究笔记");
  await send(page, "改写研究问题");
  const body = page.getByLabel("笔记正文", { exact: true });
  const argument = body.locator('.bn-block[data-id="preview-argument"] > .bn-block-content');
  const handle = (await editorOverlay(page, body)).getByRole("button", { name: "拖动块；点击打开块菜单", exact: true });
  await argument.hover({ position: { x: 16, y: 10 } });
  await expect(handle).toHaveAttribute("data-block-id", "preview-argument");
  const oldBox = await body.locator(".note-diff-deleted-blocks").boundingBox();
  const handleBox = await handle.boundingBox();
  if (!oldBox || !handleBox) throw new Error("Missing review side-menu geometry");
  expect(Math.abs(handleBox.y - oldBox.y)).toBeLessThanOrEqual(2);
  await handle.click();
  await expect(page.getByRole("menu")).toBeVisible();
  await page.keyboard.press("Escape");

  const source = body.locator('.bn-block[data-id="preview-source"] > .bn-block-content');
  await source.hover({ position: { x: 16, y: 10 } });
  await expect(handle).toHaveAttribute("data-block-id", "preview-source");
  const sourceBox = await source.boundingBox(), ordinaryHandleBox = await handle.boundingBox();
  if (!sourceBox || !ordinaryHandleBox) throw new Error("Missing ordinary block geometry");
  expect(Math.abs(ordinaryHandleBox.y - sourceBox.y)).toBeLessThanOrEqual(2);

  await page.getByRole("button", { name: "打开对话", exact: true }).click();
  await send(page, "补充验证清单");
  const addition = body.locator('.bn-block[data-id="preview-check-one"] > .bn-block-content');
  await addition.hover({ position: { x: 16, y: 10 } });
  await expect(handle).toHaveAttribute("data-block-id", "preview-check-one");
  const addedBox = await addition.boundingBox(), addedHandleBox = await handle.boundingBox();
  if (!addedBox || !addedHandleBox) throw new Error("Missing added block geometry");
  expect(Math.abs(addedHandleBox.y - addedBox.y)).toBeLessThanOrEqual(2);
  expect(errors).toEqual([]);
});
