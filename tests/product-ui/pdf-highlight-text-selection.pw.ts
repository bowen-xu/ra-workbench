import { expect, test } from "@playwright/test";

test("whole-word and drag selection work through saved highlights; Cmd-click opens their note", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  await page.goto("/?preview=app");
  await page.getByRole("row", { name: /Synthetic Research Preview/ }).dblclick();
  const line = page.locator(".pdf-view .textLayer span").filter({ hasText: "This PDF is a local sample for frontend interaction testing." }).first();
  await expect(line).toBeVisible();
  const bounds = await line.boundingBox();
  if (!bounds) throw new Error("PDF sample line is missing");
  const point = { x: bounds.x + 7, y: bounds.y + bounds.height / 2 };
  const actions = page.locator(".reader-selection-popover .selection-action-card");
  const note = page.getByRole("dialog", { name: "阅读笔记编辑面板", exact: true });
  const selectedText = () => page.locator(".pdf-view").evaluate(viewer => {
    const clipboardData = new DataTransfer();
    viewer.dispatchEvent(new ClipboardEvent("copy", { bubbles: true, cancelable: true, clipboardData }));
    return clipboardData.getData("text/plain");
  });
  // Ordinary text and saved highlights must use identical word boundaries.
  await page.mouse.dblclick(point.x, point.y);
  await expect(actions).toBeVisible();
  await actions.getByRole("button", { name: "写笔记", exact: true }).hover();
  await expect(page.getByRole("tooltip")).toHaveText("写笔记");
  await expect(page.getByRole("tooltip").locator("kbd")).toHaveCount(0);
  await expect.poll(selectedText).toBe("This");
  const before = await page.locator(".saved-highlight").count();
  await actions.getByRole("button", { name: "高亮", exact: true }).click();
  await expect(page.locator(".saved-highlight")).toHaveCount(before + 1);
  const highlight = page.locator('.saved-highlight[aria-label="This"]');
  await expect(highlight).toBeVisible();
  await highlight.dblclick();
  await expect(note).toHaveCount(0);
  await expect(highlight).toHaveAttribute("aria-pressed", "false");
  await expect(actions.getByRole("button", { name: "写笔记", exact: true })).toBeVisible();
  await expect.poll(selectedText).toBe("This");
  await page.locator(".pdf-view .page").first().click({ position: { x: 15, y: 15 } });

  // Starting inside the saved mark can extend into unhighlighted text.
  await page.mouse.move(bounds.x + 1, point.y); await page.mouse.down();
  await page.mouse.move(bounds.x + bounds.width * .5, point.y, { steps: 10 }); await page.mouse.up();
  await expect(note).toHaveCount(0);
  await expect.poll(selectedText).toMatch(/^This PDF is a local/);
  await expect(page.locator(".saved-highlight")).toHaveCount(before + 1);
  await highlight.click({ modifiers: ["Meta"] });
  await expect(note).toBeVisible(); await expect(actions).toHaveCount(0);
  await note.getByRole("button", { name: "关闭阅读笔记", exact: true }).click();
  await highlight.click();
  await expect(actions.getByRole("button", { name: "编辑笔记", exact: true })).toBeVisible();
  await actions.getByRole("button", { name: "编辑笔记", exact: true }).hover();
  await expect(page.getByRole("tooltip").locator(".control-hint-content")).toHaveText("编辑笔记");
  await expect(page.getByRole("tooltip").locator("kbd")).toHaveText(/⌘\s*\+/u);
  await expect(page.getByRole("tooltip").getByRole("img", { name: "鼠标左键" })).toBeVisible();
  await expect(note).toHaveCount(0);
  await highlight.focus(); await highlight.press("Enter"); await expect(note).toBeVisible();
  await expect(page.locator(".saved-highlight")).toHaveCount(before + 1);
  expect(errors).toEqual([]);
});
