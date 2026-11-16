import {expect, test, type Locator, type Page} from "@playwright/test";

const handleName = "拖动块；点击打开块菜单";

async function mount(page: Page): Promise<void> {
  await page.goto("/");
  await page.evaluate(async () => {
    const {mountNoteBlockActionsFixture} = await import("/ra-workbench/apps/web/src/core/components/block-editor/NoteBlockActions.fixture.tsx");
    mountNoteBlockActionsFixture();
  });
}

async function handleFor(page: Page, host: Locator, blockId: string): Promise<Locator> {
  await host.locator(`.bn-block[data-id="${blockId}"] > .bn-block-content`).hover();
  const owner = await host.getAttribute("data-editor-instance") ?? await host.locator(".block-editor").getAttribute("data-editor-instance");
  const handle = page.locator(`[data-editor-overlay-owner="${owner}"]`).getByRole("button", {name: handleName, exact: true});
  await expect(handle).toHaveAttribute("data-block-id", blockId);
  return handle;
}

test("a table block menu dismisses when clicking elsewhere in the focused editor", async ({page}) => {
  await mount(page);
  for (const name of ["普通笔记", "写作笔记"]) {
    const host = page.getByRole("region", {name, exact: true});
    await host.locator("td").first().click();
    await expect(host.locator(".bn-editor")).toBeFocused();
    const handle = await handleFor(page, host, "table");
    await handle.click();
    const menu = page.getByRole("menu", {name: "块操作", exact: true});
    await expect(menu).toBeVisible();
    await expect(menu.getByText("复制块链接", {exact: true})).toBeVisible();
    await page.screenshot({path: test.info().outputPath(`${name}-table-menu.png`)});
    await host.locator('.bn-block[data-id="tail"] > .bn-block-content').click();
    await expect(menu).toHaveCount(0);
    await expect(host.locator(".bn-editor")).toBeFocused();
    const nextHandle = await handleFor(page, host, "tail");
    await nextHandle.click();
    await expect(menu).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(menu).toHaveCount(0);
  }
});

test("a table only offers supported block actions", async ({page}) => {
  await mount(page);
  const host = page.getByRole("region", {name: "普通笔记", exact: true});
  await host.locator("td").first().click();
  await (await handleFor(page, host, "table")).click();
  const menu = page.getByRole("menu", {name: "块操作", exact: true});
  await expect(menu).toBeVisible();
  await expect(menu.getByText("转换为", {exact: true})).toHaveCount(0);
  await expect(menu.getByText("复制块链接", {exact: true})).toBeVisible();
});

test("text conversion still opens, converts and dismisses through the shared block handle", async ({page}) => {
  await mount(page);
  const host = page.getByRole("region", {name: "普通笔记", exact: true});
  await host.locator('.bn-block[data-id="tail"] > .bn-block-content').click();
  const handle = await handleFor(page, host, "tail");
  await handle.click();
  const menu = page.getByRole("menu", {name: "块操作", exact: true});
  await menu.getByRole("menuitem", {name: "转换为", exact: true}).click();
  const conversions = page.getByRole("menu", {name: "转换块类型", exact: true});
  await expect(conversions).toBeVisible();
  await page.screenshot({path: test.info().outputPath("text-conversion-menu.png")});
  await conversions.getByRole("menuitemradio", {name: "二级标题", exact: true}).click();
  await expect(host.locator('.bn-block[data-id="tail"] h2')).toHaveText("Outside menu");
  await expect(menu).toHaveCount(0);
  await expect(host.locator(".bn-editor")).toBeFocused();
});

for (const url of ["/", "/?preview=app"]) {
  test(`the application table menu closes on a body click at ${url}`, async ({page}) => {
    page.setDefaultTimeout(8000);
    await page.goto(url);
    await page.getByRole("button", {name: "笔记", exact: true}).click();
    await page.locator(".notes-explorer").getByRole("button", {name: "新建独立笔记", exact: true}).click();
    const host = page.getByRole("group", {name: "笔记正文", exact: true});
    await host.locator('[data-content-type="paragraph"]').first().click();
    await page.keyboard.insertText("Outside menu"); await page.keyboard.press("Enter");
    await page.keyboard.type("/");
    await page.getByRole("listbox").getByText("表格", {exact: true}).click();
    const table = host.locator('[data-content-type="table"]');
    await expect(table).toBeVisible();
    const id = await table.evaluate(element => element.closest(".bn-block[data-id]")!.getAttribute("data-id")!);
    await table.locator("td").first().click();
    await (await handleFor(page, host, id)).click();
    const menu = page.getByRole("menu", {name: "块操作", exact: true});
    await expect(menu).toBeVisible();
    await expect(menu.getByText("转换为", {exact: true})).toHaveCount(0);
    await page.screenshot({path: test.info().outputPath("application-table-menu.png")});
    await host.locator('[data-content-type="paragraph"]').filter({hasText: "Outside menu"}).click();
    await expect(menu).toHaveCount(0);
    await expect(host.locator(".bn-editor")).toBeFocused();
  });
}
