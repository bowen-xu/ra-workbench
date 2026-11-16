import { expect, test } from "@playwright/test";

test("full App selects highlights without outlines, opens notes on Cmd-click and retains saved changes", async ({ page }) => {
  const failures: string[] = [];
  page.on("pageerror", error => failures.push(error.message));
  await page.route("**/*", route => {
    const url = new URL(route.request().url());
    if (url.origin !== new URL(test.info().project.use.baseURL as string).origin || /^\/(api|v1|rpc|tauri)(\/|$)/.test(url.pathname)) {
      failures.push(`Forbidden request: ${url.href}`); return route.abort();
    }
    return route.continue();
  });
  await page.goto("/?preview=app");
  await page.getByRole("row", { name: /Synthetic Research Preview/ }).dblclick();
  const line = page.locator(".pdf-view .textLayer span").filter({ hasText: "This PDF is a local sample for frontend interaction testing." }).first();
  await expect(line).toBeVisible();
  const highlights = page.locator(".saved-highlight");
  await expect(highlights).toHaveCount(1); await expect(highlights.first()).toBeVisible();
  const bounds = await line.boundingBox(); if (!bounds) throw new Error("PDF glyph line is missing");
  await page.mouse.move(bounds.x + 2, bounds.y + bounds.height / 2); await page.mouse.down();
  await page.mouse.move(bounds.x + Math.min(bounds.width - 1, 250), bounds.y + bounds.height / 2, { steps: 8 }); await page.mouse.up();
  const actions = page.locator(".reader-selection-popover .selection-action-card");
  await expect(actions).toBeVisible();
  await page.getByRole("banner", { name: "窗口工具栏", exact: true }).click({ position: { x: 115, y: 3 } });
  await expect(actions).toBeVisible();
  await page.getByRole("tab", { name: "高亮与笔记", exact: true }).click();
  await page.locator(".reader-notes-heading").click();
  await expect(actions).toBeVisible();
  await page.locator(".pdf-view .page").first().click({ position: { x: 15, y: 15 } });
  await expect(actions).toHaveCount(0);
  await page.mouse.move(bounds.x + 2, bounds.y + bounds.height / 2); await page.mouse.down();
  await page.mouse.move(bounds.x + Math.min(bounds.width - 1, 250), bounds.y + bounds.height / 2, { steps: 8 }); await page.mouse.up();
  await expect(actions).toBeVisible();
  await expect(actions.locator("blockquote")).toHaveCount(0);
  await expect(actions.getByRole("button", { name: "高亮", exact: true })).toHaveText("");
  const popover = page.locator(".reader-selection-popover");
  const toolbarBounds = await popover.boundingBox();
  expect(toolbarBounds!.width).toBe(236); expect(toolbarBounds!.height).toBeLessThanOrEqual(40);
  expect(await actions.getByRole("button").evaluateAll(buttons => buttons.every(button => {
    const bounds = button.getBoundingClientRect(), parent = button.closest(".reader-selection-popover")!.getBoundingClientRect();
    return bounds.left >= parent.left && bounds.right <= parent.right;
  }))).toBe(true);
  const apply = actions.getByRole("button", { name: "高亮", exact: true });
  const options = actions.getByRole("button", { name: "选择高亮颜色", exact: true });
  const split = actions.locator(".selection-highlight-control");
  await expect(apply.locator("svg")).toHaveCSS("width", "11px");
  await apply.hover(); await expect(split).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await expect(apply).toHaveCSS("background-color", "rgb(223, 229, 223)");
  await options.hover(); await expect(split).toHaveCSS("background-color", "rgb(223, 229, 223)");
  await expect(apply).toHaveCSS("background-color", "rgba(0, 0, 0, 0)");
  await actions.getByRole("button", { name: "选择高亮颜色", exact: true }).click();
  const palette = page.getByRole("menu", { name: "高亮预设颜色", exact: true });
  await expect(palette).toBeVisible();
  await palette.press("Escape"); await expect(palette).not.toBeVisible(); await expect(actions).toBeVisible();
  await actions.getByRole("button", { name: "选择高亮颜色", exact: true }).click();
  await palette.getByRole("menuitemradio", { name: "绿色高亮", exact: true }).hover();
  await expect(page.getByRole("tooltip")).toHaveText("绿色高亮");
  await page.screenshot({ path: ".tmp/pdf-selection-icon-actions.png" });
  await palette.getByRole("menuitemradio", { name: "绿色高亮", exact: true }).click();
  await expect(palette).not.toBeVisible(); await expect(highlights).toHaveCount(1);
  await expect(actions.locator(".selection-highlight-apply .selection-color-swatch")).toHaveCSS("background-color", "rgb(145, 213, 163)");
  await page.screenshot({ path: ".tmp/pdf-selection-compact-toolbar.png" });
  await actions.getByRole("button", { name: "高亮", exact: true }).click();
  await expect(highlights).toHaveCount(2);
  const saved = highlights.last(); await expect(saved).toBeVisible();
  await expect(saved).toHaveCSS("background-color", "rgb(145, 213, 163)");
  expect(await saved.evaluate(node => {
    const b = node.getBoundingClientRect(); return document.elementFromPoint(b.x + b.width / 2, b.y + b.height / 2) === node;
  })).toBe(true);
  await saved.click(); await expect(actions).toBeVisible();
  await page.getByRole("button", { name: "隐藏笔记侧栏", exact: true }).click();
  await saved.press("Meta+z"); await expect(highlights).toHaveCount(1); await expect(actions).toHaveCount(0);
  await page.getByRole("button", { name: "重做批注更改", exact: true }).click(); await expect(highlights).toHaveCount(2);
  await page.getByRole("button", { name: "显示笔记侧栏", exact: true }).click();
  await page.locator(".pdf-view .page").first().click({ position: { x: 15, y: 15 } });
  const beforeZoom = await saved.boundingBox();
  await page.getByRole("button", { name: "放大", exact: true }).click();
  await expect.poll(async () => (await highlights.last().boundingBox())?.width ?? 0).toBeGreaterThan(beforeZoom!.width);
  await expect(highlights).toHaveCount(2);
  const unselectedStyle = await highlights.last().evaluate(node => {
    const style = getComputedStyle(node); return { background: style.backgroundColor, zIndex: style.zIndex };
  });
  await expect(highlights.last()).toHaveCSS("opacity", "0.38");
  await highlights.last().click();
  const panel = page.getByRole("dialog", { name: "阅读笔记编辑面板", exact: true });
  await expect(panel).toHaveCount(0);
  await expect(highlights.last()).toHaveAttribute("aria-pressed", "true");
  await expect(highlights.last()).toHaveCSS("outline-style", "none");
  await expect(highlights.last()).toHaveCSS("border-width", "0px");
  await expect(highlights.last()).toHaveCSS("opacity", "0.6");
  expect(await highlights.last().evaluate(node => {
    const style = getComputedStyle(node); return { background: style.backgroundColor, zIndex: style.zIndex };
  })).toEqual(unselectedStyle);
  await expect(actions.getByRole("button", { name: "编辑笔记", exact: true })).toBeVisible();
  await expect(page.locator(".pending-selection-highlight")).toHaveCount(0);
  await expect(actions.locator(".selection-highlight-apply .selection-color-swatch")).toHaveCSS("background-color", "rgb(145, 213, 163)");
  // Page whitespace clears selection without deleting the persisted mark.
  await page.locator(".pdf-view .page").first().click({ position: { x: 15, y: 15 } });
  await expect(highlights.last()).toHaveAttribute("aria-pressed", "false");
  await expect(highlights.last()).toHaveCSS("opacity", "0.38");
  await expect(page.getByRole("button", { name: "删除到统一回收站", exact: true })).toHaveCount(0);
  await expect(highlights).toHaveCount(2);
  await highlights.last().click();
  const textBounds = await line.boundingBox(); if (!textBounds) throw new Error("PDF glyph line is missing");
  await line.click({ position: { x: textBounds.width - 2, y: textBounds.height / 2 } });
  await expect(highlights.last()).toHaveAttribute("aria-pressed", "false");
  await expect(highlights.last()).toHaveCSS("opacity", "0.38");
  await highlights.first().click();
  await expect(highlights.first()).toHaveAttribute("aria-pressed", "true");
  await expect(highlights.last()).toHaveAttribute("aria-pressed", "false");
  await highlights.last().click();
  await expect(highlights.first()).toHaveAttribute("aria-pressed", "false");
  await expect(highlights.last()).toHaveCSS("opacity", "0.6");
  await actions.getByRole("button", { name: "编辑笔记", exact: true }).hover();
  await expect(page.getByRole("tooltip").locator(".control-hint-content")).toHaveText("编辑笔记");
  await expect(page.getByRole("tooltip").locator("kbd")).toHaveText(/⌘\s*\+/u);
  await expect(page.getByRole("tooltip").getByRole("img", { name: "鼠标左键" })).toBeVisible();
  await actions.getByRole("button", { name: "选择高亮颜色", exact: true }).click();
  await palette.getByRole("menuitemradio", { name: "蓝色高亮", exact: true }).click();
  await actions.getByRole("button", { name: "高亮", exact: true }).click();
  await expect(highlights).toHaveCount(2); await expect(highlights.last()).toHaveCSS("background-color", "rgb(145, 189, 232)");
  await page.getByRole("button", { name: "撤销批注更改", exact: true }).click();
  await expect(highlights.last()).toHaveCSS("background-color", "rgb(145, 213, 163)");
  await highlights.last().click(); await actions.getByRole("button", { name: "编辑笔记", exact: true }).click();
  await expect(panel).toBeVisible(); await expect(actions).toHaveCount(0);
  await panel.getByRole("button", { name: "关闭阅读笔记", exact: true }).click();
  await expect(highlights).toHaveCount(2); await highlights.last().click();
  // Compare actual rendered pixels with the overlay hidden: dark PDF ink must
  // stay dark, while paper receives the marker color. CSS assertions alone
  // cannot detect a child blend trapped inside an isolated stacking context.
  const rect = await highlights.last().boundingBox(); if (!rect) throw new Error("Saved highlight bounds are missing");
  const highlighted = await page.screenshot({ clip: rect });
  await highlights.last().evaluate(node => { (node as HTMLElement).style.visibility = "hidden"; });
  const plain = await page.screenshot({ clip: rect });
  await highlights.last().evaluate(node => { (node as HTMLElement).style.removeProperty("visibility"); });
  const pixels = await page.evaluate(async ({ plain, highlighted }) => {
    const decode = async (base64: string) => {
      const image = new Image(); image.src = `data:image/png;base64,${base64}`; await image.decode();
      const canvas = document.createElement("canvas"); canvas.width = image.width; canvas.height = image.height;
      const context = canvas.getContext("2d")!; context.drawImage(image, 0, 0); return context.getImageData(0, 0, image.width, image.height).data;
    };
    const [before, after] = await Promise.all([decode(plain), decode(highlighted)]);
    let ink = 0, washedOut = 0, coloredPaper = 0;
    for (let i = 0; i < before.length; i += 4) {
      if (before[i]! < 40 && before[i + 1]! < 40 && before[i + 2]! < 40) {
        ink++; if (after[i]! > before[i]! + 8 || after[i + 1]! > before[i + 1]! + 8 || after[i + 2]! > before[i + 2]! + 8) washedOut++;
      }
      if (before[i]! > 250 && before[i + 1]! > 250 && before[i + 2]! > 250 && after[i + 2]! < 240) coloredPaper++;
    }
    return { ink, washedOut, coloredPaper };
  }, { plain: plain.toString("base64"), highlighted: highlighted.toString("base64") });
  expect(pixels.ink).toBeGreaterThan(20); expect(pixels.washedOut).toBe(0); expect(pixels.coloredPaper).toBeGreaterThan(20);
  await page.screenshot({ path: ".tmp/pdf-highlight-selected.png" });
  // Neither editable controls nor focus elsewhere in the app may delete marks.
  const pageInput = page.getByRole("textbox", { name: "页码", exact: true });
  await pageInput.focus(); await pageInput.press("Backspace"); await expect(highlights).toHaveCount(2);
  await pageInput.press("Control+z"); await expect(highlights).toHaveCount(2);
  const outside = page.getByRole("button", { name: "收起主导航", exact: true });
  await outside.focus(); await outside.press("Delete"); await outside.press("Control+z"); await expect(highlights).toHaveCount(2);
  await highlights.last().click(); await highlights.last().press("Delete");
  await expect(highlights).toHaveCount(1); await expect(panel).toHaveCount(0);
  await expect(actions).toHaveCount(0);
  const undo = page.locator(".reader-toolbar").getByRole("button", { name: "撤销批注更改", exact: true });
  await undo.focus(); await undo.press("Meta+z");
  await expect(highlights).toHaveCount(2);
  await undo.press("Meta+Shift+z"); await expect(highlights).toHaveCount(1);
  await undo.press("Meta+z"); await expect(highlights).toHaveCount(2);
  await highlights.last().click(); await highlights.last().press("Backspace"); await expect(highlights).toHaveCount(1);
  await undo.click();
  await expect(highlights).toHaveCount(2);
  const entry = page.locator(".annotation-list button").last();
  await entry.click(); await expect(entry).toHaveAttribute("aria-pressed", "true"); await expect(panel).toHaveCount(0);
  await entry.dblclick(); await expect(panel).toBeVisible();
  const editor = panel.locator('[contenteditable="true"]').first();
  await editor.focus(); await editor.press("Backspace"); await editor.press("Meta+z");
  await expect(highlights).toHaveCount(2);
  await panel.getByRole("button", { name: "关闭阅读笔记", exact: true }).click();
  await highlights.last().click({ modifiers: ["Meta"] }); await expect(panel).toBeVisible();
  await panel.getByRole("button", { name: "关闭阅读笔记", exact: true }).click();
  await highlights.last().focus(); await highlights.last().press("Space"); await expect(panel).toHaveCount(0);
  await highlights.last().press("Enter"); await expect(panel).toBeVisible();
  await panel.getByRole("button", { name: "关闭阅读笔记", exact: true }).click();
  await page.screenshot({ path: ".tmp/pdf-saved-highlights.png" });
  await page.reload(); await page.getByRole("tab", { name: "返回文献列表", exact: true }).click();
  await page.getByRole("row", { name: /Synthetic Research Preview/ }).dblclick();
  await expect(highlights).toHaveCount(2); await expect(highlights.last()).toBeVisible();
  await expect(highlights.last()).toHaveCSS("background-color", "rgb(145, 213, 163)");
  expect(failures).toEqual([]);
});
