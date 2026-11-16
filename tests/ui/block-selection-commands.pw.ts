import { expect, test, type Locator, type Page } from "@playwright/test";

async function mount(page: Page): Promise<Locator> {
  await page.goto("/");
  await page.evaluate(async () => {
    const { mountSharedEditorFixture } = await import("/src/core/components/block-editor/BlockEditor.fixture.tsx");
    mountSharedEditorFixture();
  });
  const host = page.getByRole("region", { name: "写作思路" });
  await expect(host.locator(".bn-block-content")).toHaveText(["First", "Parent", "Child", "Tail", /x/]);
  return host;
}

async function marquee(page: Page, host: Locator, firstId: string, lastId: string): Promise<void> {
  const first = await host.locator(`.bn-block[data-id="${firstId}"] > .bn-block-content`).boundingBox();
  const last = await (lastId === "formula" ? host.locator('[data-content-type="formulaBlock"]')
    : host.locator(`.bn-block[data-id="${lastId}"] > .bn-block-content`)).boundingBox();
  const editor = await host.locator(".bn-editor").boundingBox();
  if (!first || !last || !editor) throw new Error("Missing marquee geometry");
  await page.mouse.move(editor.x + editor.width - 4, first.y + 1);
  await page.mouse.down();
  await page.mouse.move(last.x + 40, last.y + last.height - 1, { steps: 12 });
  await page.mouse.up();
  await expect(host.locator(".bn-editor")).toBeFocused();
  await expect(host.locator("style[data-note-block-highlight]")).toHaveCount(1);
  await expect(host.locator(".bn-editor")).toHaveCSS("caret-color", "rgba(0, 0, 0, 0)");
  expect(await host.locator(".bn-inline-content").first().evaluate(node => getComputedStyle(node, "::selection").backgroundColor)).toBe("rgba(0, 0, 0, 0)");
}

for (const key of ["Backspace", "Delete"]) {
  test(`marquee ${key} removes whole roots and one undo restores content and selection`, async ({ page }) => {
    const errors: string[] = []; page.on("pageerror", error => errors.push(error.message));
    const host = await mount(page);
    const right = page.getByRole("region", { name: "临时草稿" });
    await marquee(page, host, "first", "parent");
    if (key === "Backspace") await page.screenshot({ path: test.info().outputPath("structural-block-selection.png") });
    await page.keyboard.press(key);
    await expect(host.locator(".bn-block-content")).toHaveText(["Tail", /x/]);
    await expect(right.locator(".bn-block-content")).toHaveText(["First", "Parent", "Child", "Tail", /x/]);
    await page.keyboard.press("ControlOrMeta+z");
    await expect(host.locator(".bn-block-content")).toHaveText(["First", "Parent", "Child", "Tail", /x/]);
    await expect(host.locator("style[data-note-block-highlight]")).toHaveCount(1);
    await page.keyboard.press("ControlOrMeta+Shift+z");
    await expect(host.locator(".bn-block-content")).toHaveText(["Tail", /x/]);
    expect(errors).toEqual([]);
  });
}

test("typing replaces the selected blocks, Escape restores a caret, and atom blocks delete as blocks", async ({ page }) => {
  const host = await mount(page);
  await marquee(page, host, "first", "parent");
  await page.keyboard.type("Replacement");
  await expect(host.locator(".bn-block-content")).toHaveText(["Replacement", "Tail", /x/]);
  await expect(host.locator("style[data-note-block-highlight]")).toHaveCount(0);
  await page.keyboard.press("ControlOrMeta+z");
  await expect(host.locator(".bn-block-content")).toHaveText(["First", "Parent", "Child", "Tail", /x/]);
  await page.keyboard.press("Escape");
  await expect(host.locator("style[data-note-block-highlight]")).toHaveCount(0);
  await marquee(page, host, "tail", "formula");
  await page.keyboard.press("Backspace");
  await expect(host.locator(".bn-block-content")).toHaveText(["First", "Parent", "Child"]);
});

test("browser text insertion and IME replacement retain disjoint, unselected content", async ({ page }) => {
  const host = await mount(page);
  await page.keyboard.down("Shift");
  for (const id of ["first", "tail"]) {
    await host.locator(`.bn-block[data-id="${id}"] > .bn-block-content`).hover();
    const owner = await host.locator(".block-editor").getAttribute("data-editor-instance");
    const handle = page.locator(`[data-editor-overlay-owner="${owner}"]`).getByRole("button", { name: "拖动块；点击打开块菜单" });
    await handle.click();
  }
  await page.keyboard.up("Shift");
  await expect(host.locator(".bn-editor")).toBeFocused();
  await expect(host.locator("style[data-note-block-highlight]")).toHaveAttribute("data-note-block-highlight", '["first","tail"]');
  await page.keyboard.insertText("插入");
  await expect(host.locator(".bn-block-content")).toHaveText(["插入", "Parent", "Child", /x/]);
  await page.keyboard.press("ControlOrMeta+z");
  await expect(host.locator("style[data-note-block-highlight]")).toHaveAttribute("data-note-block-highlight", '["first","tail"]');
  const input = await page.context().newCDPSession(page);
  const beforeComposition = await host.locator("output").textContent();
  await input.send("Input.imeSetComposition", { text: "中文", selectionStart: 2, selectionEnd: 2 });
  await expect(host.locator("output")).toHaveText(beforeComposition ?? "");
  await input.send("Input.insertText", { text: "中文" });
  await expect(host.locator(".bn-block-content")).toHaveText(["中文", "Parent", "Child", /x/]);
  await expect(host.locator("output")).toContainText("中文");
  await page.keyboard.press("ControlOrMeta+z");
  await expect(host.locator(".bn-block-content")).toHaveText(["First", "Parent", "Child", "Tail", /x/]);
});
