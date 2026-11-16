import { expect, test } from "@playwright/test";

test("writing notes use independent files and retain their editor across saved tab switches", async ({ page }) => {
  const errors: string[] = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto("/");
  await page.getByRole("button", { name: "写作", exact: true }).click();
  await page.getByRole("button", { name: "新建稿件", exact: true }).click();
  await page.getByRole("dialog", { name: "新建 LaTeX 稿件" }).getByRole("textbox").fill("Writing notes regression");
  await page.getByRole("button", { name: "创建稿件", exact: true }).click();
  await page.getByRole("tab", { name: "笔记", exact: true }).click();
  const notes = page.getByRole("region", { name: "写作笔记" });
  await expect(notes).toContainText("记录写作思路");
  await page.getByRole("button", { name: "新建笔记", exact: true }).click();
  await notes.getByRole("textbox", { name: "笔记名称" }).fill("思路");
  await notes.getByRole("textbox", { name: "笔记名称" }).press("Enter");
  const editor = page.locator(".writing-note-session:not([hidden]) .bn-editor");
  await expect(editor).toBeVisible();
  await expect(page.getByRole("toolbar", { name: "源码编辑工具" })).toBeHidden();
  await editor.click();
  await page.keyboard.insertText("Research ideas");
  await expect(page.locator(".save-state")).toHaveText("已保存");
  const dom = await editor.elementHandle();
  await page.getByRole("tablist", {name: "打开的标签页", exact: true}).getByRole("tab", { name: "main.tex", exact: true }).click();
  await expect(page.locator(".cm-content")).toBeVisible();
  await page.getByRole("tab", { name: "笔记", exact: true }).click();
  await notes.getByRole("button", { name: "思路", exact: true }).click();
  await expect(editor).toContainText("Research ideas");
  expect(await dom?.evaluate((node) => node.isConnected)).toBe(true);
  await editor.click(); await page.keyboard.press("ControlOrMeta+z");
  await expect(editor).not.toContainText("Research ideas");
  const stored = await page.evaluate(async () => {
    const { browserWritingGateway: gateway } = await import("/plugins/ra-writing-projects/src/features/writing/gateway/browserWritingGateway.ts");
    const owner = (await gateway.listManuscripts())[0];
    return owner ? gateway.readFile(owner.id, "notes/思路.ranote") : undefined;
  });
  expect(stored?.content).not.toContain('"pageId"');
  await page.locator('.writing-note-session:not([hidden]) input[type="file"]').setInputFiles({
    name: "ideas.txt", mimeType: "text/plain", buffer: Buffer.from("owned resource"),
  });
  await expect(page.locator(".writing-note-asset")).toContainText("ideas.txt");
  await expect(page.locator(".save-state")).toHaveText("已保存");
  const noteRow = notes.locator(".writing-file-row").filter({has: page.getByRole("button", {name: "思路", exact: true})});
  await noteRow.hover();
  await noteRow.getByRole("button", { name: "笔记操作 思路" }).click();
  const rename = page.getByRole("menuitem", { name: "重命名", exact: true });
  await expect.poll(() => rename.evaluate(element => {
    const box = element.getBoundingClientRect();
    const hit = document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2);
    return hit !== null && element.contains(hit);
  })).toBe(true);
  await page.screenshot({ path: test.info().outputPath("writing-note-menu-paint-order.png") });
  await rename.click();
  await notes.getByRole("textbox", { name: "笔记名称" }).fill("修改后的思路");
  await notes.getByRole("button", { name: "确定", exact: true }).click();
  await expect(notes.getByRole("button", { name: "修改后的思路", exact: true })).toBeVisible();
  await expect(page.locator(".writing-note-asset")).toContainText("ideas.txt");
  await page.setViewportSize({ width: 1000, height: 720 });
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true);
  const bounds = await page.locator(".writing-note-scroll").boundingBox();
  expect(bounds?.height).toBeGreaterThan(300);
  await page.screenshot({ path: ".tmp/playwright-results/writing-notes.png" });
  expect(errors).toEqual([]);
});
