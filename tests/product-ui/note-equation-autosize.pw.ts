import { expect, test, type Locator } from "@playwright/test";

async function geometry(input: Locator) {
  return input.evaluate(element => {
    const field = element.closest(".cm-scroller") as HTMLElement;
    const panel = field.closest(".inline-equation-editor, .note-formula-edit-panel")!;
    const style = getComputedStyle(field);
    return {
      height: field.clientHeight, content: field.scrollHeight, width: field.clientWidth,
      lineHeight: Number.parseFloat(style.lineHeight), overflow: style.overflowY,
      panelOverflow: panel.scrollHeight - panel.clientHeight,
      top: panel.getBoundingClientRect().top, bottom: panel.getBoundingClientRect().bottom,
    };
  });
}

for (const url of ["/", "/?preview=app"]) {
  for (const kind of ["inline", "block"] as const) {
    test(`${kind} equation source fits rendered lines, caps height and shrinks on ${url}`, async ({ page }) => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      await page.goto(url);
      await page.getByRole("button", { name: "笔记", exact: true }).click();
      await page.locator(".notes-explorer").getByRole("button", { name: "新建独立笔记", exact: true }).click();
      const editor = page.locator(".notes-content .bn-editor");
      await editor.click();
      await page.keyboard.type(kind === "inline" ? "$" : "$$");
      await page.keyboard.press("Tab");
      if (kind === "block") await page.locator(".note-formula-disclosure").click();
      const input = page.getByRole("textbox", {
        name: kind === "inline" ? "行内公式 LaTeX 源码" : "公式 LaTeX 源码", exact: true,
      });
      await expect(input).toBeFocused();
      await input.fill("x^2");
      const compact = await geometry(input);

      // A single logical line must also grow for visual soft wraps.
      const wrapped = String.raw`\sqrt{\frac{\log{N(s)}}{N(s,a)}} + `.repeat(6);
      await input.fill(wrapped);
      await expect.poll(async () => (await geometry(input)).height).toBeGreaterThan(compact.height);
      await expect.poll(async () => { const box = await geometry(input); return box.content - box.height; }).toBeLessThanOrEqual(1);
      await page.screenshot({ path: test.info().outputPath(`${kind}-equation-wrapped.png`) });

      // No eight-line ceiling: ten short lines remain completely visible.
      const tenLines = Array.from({ length: 10 }, () => "x^2").join("\n");
      await input.fill(tenLines);
      await expect.poll(async () => (await geometry(input)).height).toBeGreaterThan(compact.lineHeight * 8);
      await expect.poll(async () => { const box = await geometry(input); return box.content - box.height; }).toBeLessThanOrEqual(1);
      await input.press("Enter");
      const trigger = page.locator(kind === "inline" ? ".inline-equation-preview" : ".note-formula-disclosure");
      await trigger.click();
      await expect.poll(async () => (await input.locator(".cm-line").allTextContents()).join("\n")).toBe(tenLines);
      await expect.poll(async () => (await geometry(input)).height).toBeGreaterThan(compact.lineHeight * 8);

      await input.fill(Array.from({ length: 80 }, () => "x^2").join("\n"));
      await expect.poll(async () => { const box = await geometry(input); return box.content - box.height; }).toBeGreaterThan(100);
      const capped = await geometry(input);
      expect(capped.height).toBeLessThanOrEqual(360);
      expect(capped.overflow).toBe("auto");
      expect(capped.panelOverflow).toBeLessThanOrEqual(1);
      expect(capped.top).toBeGreaterThanOrEqual(0);
      expect(capped.bottom).toBeLessThanOrEqual(page.viewportSize()!.height);
      const done = page.getByRole("button", { name: "完成", exact: true });
      await expect(done).toBeInViewport();
      expect(await done.evaluate(element => {
        const box = element.getBoundingClientRect();
        return element.contains(document.elementFromPoint(box.x + box.width / 2, box.y + box.height / 2));
      })).toBe(true);
      await input.evaluate(element => { const scroll = element.closest(".cm-scroller")!; scroll.scrollTop = scroll.scrollHeight; });
      expect(await input.evaluate(element => element.closest(".cm-scroller")!.scrollTop)).toBeGreaterThan(0);
      await page.screenshot({ path: test.info().outputPath(`${kind}-equation-capped.png`) });
      await page.setViewportSize({ width: 1280, height: 400 });
      await expect.poll(async () => (await geometry(input)).height).toBeLessThan(capped.height);
      await expect.poll(async () => (await geometry(input)).bottom).toBeLessThanOrEqual(400);
      await expect(done).toBeInViewport();
      await page.setViewportSize({ width: 1280, height: 720 });

      await input.fill(wrapped);
      const wide = await geometry(input);
      await page.setViewportSize({ width: 360, height: 720 });
      await expect.poll(async () => (await geometry(input)).width).toBeLessThan(wide.width);
      await expect.poll(async () => (await geometry(input)).height).toBeGreaterThan(wide.height);
      await page.setViewportSize({ width: 1280, height: 720 });
      await expect.poll(async () => (await geometry(input)).height).toBeLessThanOrEqual(wide.height);

      await input.fill("x^2");
      await expect.poll(async () => (await geometry(input)).height).toBe(compact.height);
      await expect(input).toBeFocused();
      await input.press("End"); await input.press("Shift+Enter"); await input.press("Tab");
      await expect.poll(async () => (await input.locator(".cm-line").allTextContents()).join("\n")).toBe("x^2\n\t");
      await expect(input).toBeFocused();
      await done.click();
      await expect(input).toHaveCount(0);
      expect(errors).toEqual([]);
    });
  }
}
