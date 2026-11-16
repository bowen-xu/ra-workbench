import { expect, test, type Page } from "@playwright/test";

async function noteSnapshot(page: Page) {
  return page.evaluate(async () => {
    const path = "/ra-workbench/apps/web/src/core/features/notes/gateways/index.ts";
    const gateway = (await import(path)).getNotesGateway();
    const note = (await gateway.listPages()).find((item: { title: string }) => item.title === "Idle save note");
    if (!note) return undefined;
    const snapshot = await gateway.loadDocument(note.id);
    return { revision: snapshot.page.revision, body: JSON.stringify(snapshot.document) };
  });
}
async function sourceContent(page: Page) {
  return page.evaluate(async () => {
    const path = "/plugins/ra-writing-projects/src/features/writing/gateway/index.ts";
    const gateway = (await import(path)).getWritingGateway();
    const manuscript = (await gateway.listManuscripts()).find((item: { title: string }) => item.title === "Idle save writing");
    return manuscript ? (await gateway.readFile(manuscript.id, "main.tex")).content as string : undefined;
  });
}

for (const url of ["/", "/?preview=app"]) {
  test(`notes wait for idle and flush on workspace switch on ${url}`, async ({ page }) => {
    await page.goto(url); await page.getByRole("button", { name: "笔记", exact: true }).click();
    const editor = page.locator(".notes-content .bn-editor");
    const previous = await editor.count() ? await editor.elementHandle() : null;
    await page.locator(".notes-explorer").getByRole("button", { name: "新建页面", exact: true }).click();
    if (previous) await expect.poll(() => previous.evaluate(element => element.isConnected)).toBe(false);
    await page.getByRole("textbox", { name: "笔记标题" }).fill("Idle save note"); await editor.click();
    await expect.poll(() => noteSnapshot(page)).toBeDefined();
    const original = await noteSnapshot(page);
    for (let index = 0; index < 3; index++) {
      await page.keyboard.insertText(`idle ${index} `);
      await page.waitForTimeout(1000);
      expect((await noteSnapshot(page))?.revision).toBe(original!.revision);
    }
    await expect.poll(async () => (await noteSnapshot(page))?.body).toContain("idle 0 idle 1 idle 2");
    await page.keyboard.insertText("save before leaving");
    await page.getByRole("button", { name: "文献", exact: true }).click();
    await expect(editor).toBeHidden();
    expect((await noteSnapshot(page))?.body).toContain("save before leaving");
  });

  test(`writing waits for idle and flushes on workspace switch on ${url}`, async ({ page }) => {
    await page.goto(url); await page.getByRole("button", { name: "写作", exact: true }).click();
    await page.getByRole("button", { name: "新建稿件", exact: true }).click();
    await page.getByRole("dialog", { name: "新建 LaTeX 稿件" }).getByRole("textbox").fill("Idle save writing");
    await page.getByRole("button", { name: "创建稿件", exact: true }).click();
    const editor = page.locator(".writing-editor-column .cm-content").first();
    await expect(editor).toBeVisible();
    const original = await sourceContent(page);
    await editor.click(); await page.keyboard.press("ControlOrMeta+End");
    for (let index = 0; index < 3; index++) {
      await page.keyboard.insertText(`\n% idle ${index}`);
      await page.waitForTimeout(1000);
      expect(await sourceContent(page)).toBe(original);
    }
    await expect.poll(() => sourceContent(page)).toContain("% idle 2");
    await page.keyboard.insertText("\n% save before leaving");
    await page.getByRole("button", { name: "笔记", exact: true }).click();
    await expect(page.locator(".notes-explorer")).toBeVisible();
    expect(await sourceContent(page)).toContain("% save before leaving");
  });
}
