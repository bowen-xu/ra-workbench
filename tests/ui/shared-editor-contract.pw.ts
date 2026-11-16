import { expect, test, type Page } from "@playwright/test";

async function mount(page: Page): Promise<void> {
  await page.goto("/");
  await page.evaluate(async () => {
    const { mountSharedEditorFixture } = await import("/src/core/components/block-editor/BlockEditor.fixture.tsx");
    mountSharedEditorFixture();
  });
}

test("a minimal host gets complete menus and every typed trigger obeys the same prefix rule", async ({ page }) => {
  test.setTimeout(60_000);
  await mount(page);
  const menu = page.getByRole("listbox");
  for (const name of ["写作思路", "临时草稿"]) {
    const host = page.getByRole("region", { name });
    const initial = host.locator('.bn-block[data-id="first"] > .bn-block-content p');
    await initial.fill("");
    let row = 0;
    for (const [prefix, shouldOpen] of [["", true], ["   ", true], ["\u3000", true], ["word", false], ["word ", true], ["中文  ", true]] as const) {
      for (const trigger of ["/", "$", "$$", "@"]) {
        const text = host.locator(".bn-editor p").nth(row++);
        if (prefix) await page.keyboard.insertText(prefix);
        await expect(host.locator(".bn-editor")).toBeFocused();
        await page.keyboard.type(trigger);
        if (shouldOpen) {
          await expect(menu, name + " prefix=" + JSON.stringify(prefix) + " trigger=" + trigger).toBeVisible();
          if (trigger === "/") {
            await expect(menu.getByText("行内公式", { exact: true })).toBeVisible();
            await expect(menu.getByText("公式块", { exact: true })).toBeVisible();
          } else if (trigger === "$" || trigger === "$$") {
            await expect(menu.getByRole("option")).toHaveCount(1);
            await expect(menu.getByText(trigger === "$" ? "行内公式" : "公式块", { exact: true })).toBeVisible();
          } else await expect(menu.getByText("宿主扩展", { exact: true })).toBeVisible();
          await page.keyboard.press("Escape");
        }
        await expect(menu).toHaveCount(0);
        await expect(text).toHaveText(prefix + trigger);
        await page.keyboard.press("Enter");
      }
    }
  }
  const left = page.getByRole("region", { name: "写作思路" });
  await left.locator(".bn-editor p").nth(24).click(); await page.keyboard.type("$");
  await expect(menu.getByText("行内公式", { exact: true })).toBeVisible();
  await page.keyboard.press("Tab");
  const source = page.getByRole("textbox", { name: "行内公式 LaTeX 源码", exact: true });
  await source.fill("x"); await source.press("Enter");
  await page.keyboard.type(" $");
  await expect(menu.getByText("行内公式", { exact: true })).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(menu).toHaveCount(0);
  await expect(left.locator(".inline-equation-preview")).toHaveCount(1);

  const right = page.getByRole("region", { name: "临时草稿" });
  await right.locator(".bn-editor p").nth(24).click(); await page.keyboard.type("/code");
  await menu.getByText("代码块", { exact: true }).click();
  for (const trigger of ["/", "$", "@"]) {
    await page.keyboard.type(trigger);
    await expect(menu).toHaveCount(0);
  }
  await expect(right.locator('[data-content-type="codeBlock"]')).toContainText("/$@");
});

test("a host rerender with an inline movement policy preserves an active marquee", async ({ page }) => {
  await mount(page);
  const host = page.getByRole("region", { name: "写作思路" });
  const editor = host.locator(".bn-editor");
  const first = await host.locator('.bn-block[data-id="first"] > .bn-block-content').boundingBox();
  const next = await host.locator('.bn-block[data-id="parent"] > .bn-block-content').boundingBox();
  const box = await editor.boundingBox();
  if (!first || !next || !box) throw new Error("Missing editor geometry");
  await host.getByRole("button", { name: "延迟保存回执", exact: true }).click();
  await page.mouse.move(box.x + box.width - 4, first.y + 1); await page.mouse.down();
  await page.mouse.move(first.x + 40, first.y + first.height - 1, { steps: 5 });
  await expect(page.locator(".note-block-selection-rect")).toBeVisible();
  await expect(host.locator("output")).toHaveAttribute("data-revision", "1");
  await expect(page.locator(".note-block-selection-rect")).toBeVisible();
  await page.mouse.move(next.x + 40, next.y + next.height - 1, { steps: 5 });
  await expect(host.locator('.bn-block-outer[data-id="parent"]').first()).toHaveCSS("background-color", "rgb(234, 242, 237)");
  await page.mouse.up();
  await expect(page.locator(".note-block-selection-rect")).toHaveCount(0);
});
