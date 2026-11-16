import { expect, test, type Locator } from "@playwright/test";

async function appearance(button: Locator) {
  return button.evaluate(element => {
    const style = getComputedStyle(element.parentElement!);
    return { width: style.width, height: style.height, padding: style.padding, radius: style.borderRadius,
      background: style.backgroundColor, border: style.borderColor, shadow: style.boxShadow,
      iconWidth: element.querySelector("svg")!.getBoundingClientRect().width };
  });
}

for (const url of ["/", "/?preview=app"]) {
  test(`writing home matches reader home and returns to projects without closing files on ${url}`, async ({ page }) => {
    await page.setViewportSize({ width: 1272, height: 808 });
    await page.goto(url);
    const readerHome = page.getByRole("tab", { name: "返回文献列表", exact: true });
    await expect(readerHome).toBeVisible();
    await expect(readerHome.locator("..")).toHaveCSS("background-color", "rgb(255, 255, 255)");
    const selectedStyle = await appearance(readerHome);
    await page.getByRole("button", { name: "写作", exact: true }).click();
    const home = page.getByRole("tab", { name: "返回写作项目列表", exact: true });
    await expect(home).toHaveAttribute("aria-selected", "true");
    await expect.poll(() => appearance(home)).toEqual(selectedStyle);
    await page.getByRole("button", { name: "新建稿件", exact: true }).click();
    await page.getByRole("dialog", { name: "新建 LaTeX 稿件" }).getByRole("textbox").fill("Writing home regression");
    await page.getByRole("button", { name: "创建稿件", exact: true }).click();
    const tabs = page.getByRole("tablist", { name: "打开的标签页", exact: true });
    const sourceTab = tabs.getByRole("tab", { name: "main.tex", exact: true });
    await expect(home).toHaveAttribute("aria-selected", "false");
    await expect(tabs.getByRole("tab").first()).toHaveAttribute("aria-label", "返回写作项目列表");
    const editor = page.locator(".writing-editor-column .cm-content").first();
    await expect(editor).toBeVisible();
    const originalEditor = await editor.elementHandle();
    await editor.click(); await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.insertText("\n% preserved home tab edit");
    await home.click();
    await expect(page.getByRole("heading", { name: "我的稿件", exact: true })).toBeVisible();
    await expect(home).toHaveAttribute("aria-selected", "true");
    await expect.poll(() => appearance(home)).toEqual(selectedStyle);
    await expect(sourceTab).toBeVisible();
    await expect(sourceTab).toHaveAttribute("aria-selected", "false");
    await sourceTab.click(); await expect(editor).toBeVisible();
    await expect(editor).toContainText("preserved home tab edit");
    expect(await originalEditor!.evaluate(element => element.isConnected)).toBe(true);
    await editor.click(); await page.keyboard.press("ControlOrMeta+z");
    await expect(editor).not.toContainText("preserved home tab edit");
    await page.getByRole("button", { name: "文献", exact: true }).click();
    await expect(home).toBeHidden();
    await expect(readerHome).toBeVisible();
    if (url.includes("preview")) {
      await page.getByRole("button", { name: "写作", exact: true }).click();
      await page.screenshot({ path: ".tmp/writing-home-tab.png" });
    }
  });
}
