import { expect, test, type Page } from "@playwright/test";

async function createNote(page: Page, title: string) {
  const editor = page.locator(".notes-content .bn-editor");
  const previous = await editor.count() ? await editor.elementHandle() : null;
  await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
  if (previous) await expect.poll(() => previous.evaluate(node => node.isConnected)).toBe(false);
  await page.getByRole("textbox", { name: "笔记标题" }).fill(title);
  await editor.click();
  return editor;
}

for (const route of ["/", "/?preview=app"]) {
  test(`note source links persist and use internal navigation from anchors and toolbar on ${route}`, async ({ page }) => {
    await page.goto(route);
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await createNote(page, "Citation destination");
    await page.keyboard.type("Source destination text");
    await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
    const href = await page.evaluate(async () => {
      const url = "/ra-workbench/apps/web/src/core/features/notes/gateways/index.ts";
      const { getNotesGateway } = await import(/* @vite-ignore */ url);
      const gateway = getNotesGateway();
      const target = (await gateway.listPages()).find((note: { title: string }) => note.title === "Citation destination");
      const snapshot = await gateway.loadDocument(target.id);
      return `source://${target.id}:#block{${snapshot.document.blocks[0].id}}`;
    });
    const editor = await createNote(page, "Source citation note");
    await page.keyboard.type("Paper citation");
    await page.keyboard.press("ControlOrMeta+a");
    await page.locator('.bn-formatting-toolbar [data-test="createLink"]').click();
    const input = page.locator('input[name="url"]');
    await input.fill(href); await input.press("Enter");
    await page.keyboard.press("ArrowRight");
    const anchor = editor.getByRole("link", { name: "Paper citation", exact: true });
    await expect(anchor).toHaveAttribute("href", href);
    await expect(page.locator(".note-save-state")).toHaveAttribute("aria-label", "已保存");
    await page.reload();
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await page.getByRole("tab", { name: "页面", exact: true }).click();
    await page.getByRole("tree").getByRole("button", { name: "Source citation note", exact: true }).click();
    await expect(anchor).toHaveAttribute("href", href);
    await anchor.hover();
    await page.locator(".bn-link-toolbar").getByRole("button", { name: "编辑链接", exact: true }).click();
    await expect(input).toHaveValue(href);
    await input.press("Enter");
    await expect(anchor).toHaveAttribute("href", href);
    await page.mouse.move(10, 60);
    await anchor.click();
    await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("Citation destination");
    await page.getByRole("button", { name: "后退", exact: true }).click();
    await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("Source citation note");
    await anchor.hover();
    await page.locator(".bn-link-toolbar").getByRole("button", { name: "打开来源", exact: true }).click();
    await expect(page.getByRole("textbox", { name: "笔记标题" })).toHaveValue("Citation destination");
    expect(page.context().pages()).toHaveLength(1);
    await page.screenshot({ path: `.tmp/note-source-links-${route.includes("preview") ? "preview" : "app"}.png` });
  });
}
