import { expect, test, type Page } from "@playwright/test";

async function noteSelection(page: Page, url: string) {
  await page.goto(url);
  await page.getByRole("button", { name: "笔记", exact: true }).click();
  const editor = page.locator(".notes-content .bn-editor");
  const previous = await editor.count() ? await editor.elementHandle() : null;
  await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
  if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
  await expect(editor).toBeVisible();
  await page.getByRole("textbox", { name: "笔记标题" }).fill("Ask AI note selection");
  await editor.click(); await page.keyboard.type("Alpha evidence"); await page.keyboard.press("Enter"); await page.keyboard.type("Beta evidence");
  await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
  await page.keyboard.press("ControlOrMeta+a");
  const toolbar = page.locator(".bn-formatting-toolbar");
  await expect(toolbar).toHaveCount(1); await expect(toolbar).toBeVisible();
  const ask = toolbar.getByRole("button", { name: "问AI", exact: true });
  await expect(ask.locator(".lucide-message-square-quote")).toHaveCount(1);
  await expect(ask).toHaveCSS("height", "28px");
  await ask.click();
  const input = page.getByRole("textbox", { name: "批注 笔记选区", exact: true });
  await expect(input).toBeFocused();
  await expect(toolbar).toHaveCount(0);
  await expect(page.getByRole("button", { name: "打开对话", exact: true })).toBeVisible();
  await expect(page.locator(".agent-evidence-disclosure")).toHaveCount(0);
  return { editor, input };
}

for (const url of ["/", "/?preview=app"]) {
  test(`note toolbar asks AI through the shared local comment editor and drafts exact multiblock evidence on ${url}`, async ({ page }) => {
    await page.setViewportSize({ width: 1276, height: 808 });
    const { editor, input } = await noteSelection(page, url);
    await input.fill("How are these related?");
    await input.dispatchEvent("keydown", { key: "Enter", keyCode: 229, isComposing: true });
    await expect(input).toBeVisible();
    await expect(page.getByRole("button", { name: "打开对话", exact: true })).toBeVisible();
    if (url.includes("preview")) await page.screenshot({ path: ".tmp/note-selection-ask-ai.png" });
    await input.press("Enter"); await expect(input).toHaveCount(0);
    const clear = page.getByRole("button", { name: "清除全部批注", exact: true });
    await page.locator(".agent-evidence-group").hover();
    await expect(clear).toHaveCSS("opacity", "1");
    await expect(clear).toHaveCSS("border-width", "0px");
    const groupBox=(await page.locator(".agent-evidence-group").boundingBox())!;
    for (const item of [clear,clear.locator("svg")]) {
      const box=(await item.boundingBox())!;
      expect(Math.abs(box.y+box.height/2-groupBox.y-groupBox.height/2)).toBeLessThan(.5);
    }
    if (url.includes("preview")) await page.locator(".writing-agent-panel:visible").screenshot({ path: ".tmp/agent-evidence-clear-borderless.png" });
    await page.getByRole("button", { name: "2 条批注", exact: true }).click();
    const evidence = page.getByRole("dialog", { name: "批注引用内容", exact: true });
    await expect(evidence.locator(".agent-evidence-card")).toHaveCount(2);
    await expect(evidence).toContainText("Alpha evidence"); await expect(evidence).toContainText("Beta evidence");
    await expect(evidence.locator(".agent-evidence-comment")).toHaveText(["How are these related?", "How are these related?"]);
    await expect(editor.locator(".bn-block-content")).toHaveText(["Alpha evidence", "Beta evidence"]);
  });

  test(`outside and Escape silently capture note comments without opening chat or changing text on ${url}`, async ({ page }) => {
    const { editor, input } = await noteSelection(page, url);
    await input.fill("Keep this comment");
    await page.getByRole("textbox", { name: "笔记标题" }).click();
    await expect(input).toHaveCount(0);
    const launcher = page.getByRole("button", { name: "打开对话", exact: true });
    await expect(launcher).toBeVisible(); await expect(launcher).toHaveAttribute("aria-description", /草稿含/);
    await expect(page.getByRole("textbox", { name: "笔记标题" })).toBeFocused();
    await editor.click(); await page.keyboard.press("ControlOrMeta+a");
    await page.locator(".bn-formatting-toolbar").getByRole("button", { name: "问AI", exact: true }).click();
    await expect(input).toBeFocused(); await input.fill("Escape comment"); await input.press("Escape");
    await expect(input).toHaveCount(0); await expect(launcher).toBeVisible();
    await launcher.click();
    await page.getByRole("button", { name: "4 条批注", exact: true }).click();
    const evidence = page.getByRole("dialog", { name: "批注引用内容", exact: true });
    await expect(evidence.locator(".agent-evidence-comment")).toHaveText(["Keep this comment", "Keep this comment", "Escape comment", "Escape comment"]);
    await expect(editor.locator(".bn-block-content")).toHaveText(["Alpha evidence", "Beta evidence"]);
  });
}

test("PDF and note selections share the Ask AI icon and local comment behavior", async ({ page }) => {
  await page.goto("/?preview=app");
  await page.getByRole("row", { name: /Synthetic Research Preview/ }).dblclick();
  const line = page.locator(".pdf-view .textLayer span").filter({ hasText: "This PDF is a local sample for frontend interaction testing." }).first();
  await expect(line).toBeVisible(); await line.dblclick();
  const ask = page.getByLabel("选区操作", { exact: true }).getByRole("button", { name: "问AI", exact: true });
  await expect(ask.locator(".lucide-message-square-quote")).toHaveCount(1); await ask.click();
  const input = page.getByRole("textbox", { name: "批注 PDF 选区", exact: true });
  await expect(input).toBeFocused();
  await expect(page.getByRole("button", { name: "打开对话", exact: true })).toBeVisible();
  await input.fill("Shared PDF behavior"); await input.press("Escape");
  await expect(input).toHaveCount(0);
  await expect(page.getByRole("button", { name: "打开对话", exact: true })).toHaveAttribute("aria-description", /草稿含/);
});
