import { expect, test, type Locator } from "@playwright/test";

async function appearance(reference: Locator) {
  return reference.evaluate(element => {
    const css = getComputedStyle(element);
    return { color: css.color, background: css.backgroundColor, padding: css.padding,
      radius: css.borderRadius, maxWidth: css.maxWidth, decoration: css.textDecorationLine,
      iconRatio: element.querySelector('svg')!.getBoundingClientRect().width / parseFloat(css.fontSize) };
  });
}

async function createNoteFixture(page: import("@playwright/test").Page, title: string): Promise<string> {
  return page.evaluate(async title => (await import("/integration/web/src/app/previews/agent/AgentPreview.browser.tsx")).createPreviewNote(title), title);
}

async function openNoteFixture(page: import("@playwright/test").Page, id: string) {
  await page.getByRole("button", {name: "笔记", exact: true}).click();
  await page.evaluate(id => window.dispatchEvent(new CustomEvent("research-assistant:open-note-page", {detail: id})), id);
}

for (const url of ["/", "/?preview=app"]) {
  test(`note references configure note presentation while preserving chat labels and reference identity on ${url}`, async ({ page }) => {
    await page.goto('/?demo=writing-editor-diff');
    await page.getByRole('button', { name: '添加上下文和工具', exact: true }).click();
    await page.getByRole('menuitem', { name: '文件', exact: true }).click();
    await page.getByRole('menuitem', { name: /main\.tex/ }).first().click();
    const chat = page.locator('.agent-composer-reference');
    await expect(chat).toBeVisible();
    const chatStyle = await appearance(chat);
    await chat.hover();
    const chatHover = await appearance(chat);
    await page.goto(url);
    await page.getByRole('button', { name: '笔记', exact: true }).click();
    const title = 'Reference target with a long name that must retain its complete identity';
    const editor = page.locator('.notes-content:visible .bn-editor');
    await createNoteFixture(page, title);
    const sourceId = await createNoteFixture(page, "Reference source");
    await page.reload();
    await openNoteFixture(page, sourceId);
    await editor.click(); await page.keyboard.type('Before @Reference target');
    await page.getByRole('listbox').getByRole('option', { name: /^页面 · Reference target with a long name/ }).click();
    await page.keyboard.type(' After');
    const reference = editor.locator('.research-mention');
    await expect(reference).toHaveText(title);
    await expect(reference).toHaveAttribute('title', title);
    await expect(reference).toHaveAttribute('aria-label', title);
    await expect(reference).toHaveAttribute('data-target-id', /.+/);
    await expect(reference.locator('svg')).toBeVisible();
    await expect(reference).toHaveCSS("color", "rgb(47, 44, 39)");
    await expect(reference).toHaveCSS("max-width", "none");
    await expect(reference.locator("strong")).toHaveText(title);
    expect((await appearance(reference)).iconRatio).toBe(chatStyle.iconRatio);
    const label = reference.locator('.inline-reference-label');
    await expect(label).toHaveCSS("text-overflow", "clip");
    await expect(label).toHaveCSS("text-decoration-style", "dashed");
    await expect(editor.locator('.bn-block-content:not([data-content-type="documentTitle"])')).toHaveCount(1);
    await expect(editor).toContainText('Before'); await expect(editor).toContainText('After');
    await reference.hover();
    await expect(reference).toHaveCSS("background-color", "rgb(240, 239, 237)");
    expect((await appearance(reference)).color).not.toBe(chatHover.color);
    await page.mouse.move(10, 10);
    await expect(reference).toHaveCSS('background-color', 'rgba(0, 0, 0, 0)');
    if (url.includes('preview')) await page.screenshot({ path: '.tmp/side-note-reference-style.png' });
  });
}
